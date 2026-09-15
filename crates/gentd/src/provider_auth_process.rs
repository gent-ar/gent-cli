use std::{
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gent_drivers::{
    ProcessLauncher,
    interrupt::{ProcessTreeControl, ProcessTreeSignal},
    lock::capture,
    process::SystemProcess,
    supervisor::{LaunchIntent, ProviderLaunch, ProviderProcess},
};
use gent_types::{
    AgentChatProvider, ProviderAuthBinaryLock, ProviderAuthLifecycle, ProviderAuthProvider,
    RunVersionLock, SandboxWorkspaceAccess,
};
use sha2::{Digest, Sha256};

use crate::provider_launch_budget::{ProviderLaunchError, launch_budget};

const AUTH_OUTPUT_BYTES: usize = 64 * 1024;
pub(super) const PROBE_TIMEOUT: Duration = launch_budget(Duration::from_secs(5));
const VERSION_OUTPUT_BYTES: usize = 1024;
const CODEX_SIGNED_OUT: &str = "Not logged in";

pub(super) fn identified_lock(
    provider: ProviderAuthProvider,
    executable: &Path,
    timeout: Duration,
) -> Result<RunVersionLock, ProviderLaunchError> {
    let failed = |message: &str| ProviderLaunchError::Failed(message.into());
    let name = provider_name(provider);
    let before = capture(name, executable, "unprobed", "provider-auth")
        .map_err(|_| failed("provider executable could not be locked"))?;
    let process = launch(provider, &before, &["--version"]).map_err(|error| failed(&error))?;
    let success = exit_code_bounded(&process, timeout)? == Some(0);
    let output = process.output().stdout.bytes;
    if !success {
        return Err(failed("provider version probe failed"));
    }
    let version = String::from_utf8(output).map_err(|_| failed("provider version probe failed"))?;
    let version = version.trim();
    if version.is_empty() || version.len() > 128 || version.chars().any(char::is_control) {
        return Err(failed("provider version probe failed"));
    }
    let lock = capture(
        name,
        Path::new(&before.canonical_path),
        version,
        "provider-auth",
    )
    .map_err(|_| failed("provider executable could not be locked"))?;
    (before.canonical_path == lock.canonical_path
        && before.file_identity == lock.file_identity
        && before.digest_sha256 == lock.digest_sha256)
        .then_some(lock)
        .ok_or_else(|| failed("provider executable changed during version probe"))
}

pub(super) fn authentication(
    provider: ProviderAuthProvider,
    lock: &RunVersionLock,
    timeout: Duration,
) -> Result<ProviderAuthLifecycle, ProviderLaunchError> {
    let arguments = match provider {
        ProviderAuthProvider::Claude => &["auth", "status", "--json"][..],
        ProviderAuthProvider::Codex => &["login", "status"][..],
    };
    let Ok(process) = launch(provider, lock, arguments) else {
        return Ok(ProviderAuthLifecycle::Failed);
    };
    let code = match exit_code_bounded(&process, timeout) {
        Ok(code) => code,
        Err(timed_out @ ProviderLaunchError::TimedOut(_)) => return Err(timed_out),
        Err(ProviderLaunchError::Failed(_)) => return Ok(ProviderAuthLifecycle::Failed),
    };
    let output = process.output();
    Ok(classify(
        provider,
        code,
        &output.stdout.bytes,
        &output.stderr.bytes,
    ))
}

pub(super) fn classify(
    provider: ProviderAuthProvider,
    code: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
) -> ProviderAuthLifecycle {
    let signed_in = match provider {
        ProviderAuthProvider::Claude => serde_json::from_slice::<serde_json::Value>(stdout)
            .ok()
            .and_then(|value| value.get("loggedIn").and_then(serde_json::Value::as_bool))
            .filter(|signed_in| !signed_in || code == Some(0)),
        ProviderAuthProvider::Codex if code == Some(0) => Some(true),
        ProviderAuthProvider::Codex => [stdout, stderr]
            .iter()
            .any(|stream| String::from_utf8_lossy(stream).trim() == CODEX_SIGNED_OUT)
            .then_some(false),
    };
    match signed_in {
        Some(true) => ProviderAuthLifecycle::Authenticated,
        Some(false) => ProviderAuthLifecycle::Unauthenticated,
        None => ProviderAuthLifecycle::Failed,
    }
}

fn exit_code_bounded(
    process: &SystemProcess,
    timeout: Duration,
) -> Result<Option<i32>, ProviderLaunchError> {
    let deadline = Instant::now() + timeout;
    loop {
        match process.try_exit_code() {
            Ok(Some(code)) => return Ok(code),
            Err(_) => {
                return Err(ProviderLaunchError::Failed(
                    "provider authentication probe failed".into(),
                ));
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = process.signal_tree(ProcessTreeSignal::Kill);
                return Err(ProviderLaunchError::TimedOut(
                    "provider authentication probe timed out".into(),
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
        }
    }
}

pub(super) fn launch(
    provider: ProviderAuthProvider,
    lock: &RunVersionLock,
    arguments: &[&str],
) -> Result<SystemProcess, String> {
    let limit = if arguments == ["--version"] {
        VERSION_OUTPUT_BYTES
    } else {
        AUTH_OUTPUT_BYTES
    };
    let process = crate::node_runtime_lock::standalone_provider_launcher(limit)
        .launch(&ProviderLaunch {
            lock: lock.clone(),
            provider: provider_name(provider).into(),
            executable: PathBuf::from(&lock.canonical_path),
            arguments: arguments.iter().map(|value| (*value).into()).collect(),
            intent: LaunchIntent::Start,
            workspace_root: None,
            workspace_access: SandboxWorkspaceAccess::ReadOnly,
        })
        .map_err(|_| "provider authentication process could not start".to_owned())?;
    process
        .close_stdin()
        .map_err(|_| "provider authentication process could not start".to_owned())?;
    Ok(process)
}

pub(super) fn login_arguments(provider: ProviderAuthProvider) -> &'static [&'static str] {
    match provider {
        ProviderAuthProvider::Claude => &["auth", "login"],
        ProviderAuthProvider::Codex => &["login"],
    }
}

pub(super) fn public_lock(
    provider: ProviderAuthProvider,
    lock: &RunVersionLock,
) -> ProviderAuthBinaryLock {
    ProviderAuthBinaryLock {
        canonical_executable_id: format!("provider-id:{}", provider_name(provider)),
        digest_sha256: lock.digest_sha256.clone(),
        version: lock.version.clone(),
    }
}

pub(super) fn challenge_id(
    provider: ProviderAuthProvider,
    request_id: &str,
    digest: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider_name(provider));
    hasher.update(request_id);
    hasher.update(digest);
    format!("auth-{:x}", hasher.finalize())
}

const fn provider_name(provider: ProviderAuthProvider) -> &'static str {
    match provider {
        ProviderAuthProvider::Claude => "claude",
        ProviderAuthProvider::Codex => "codex",
    }
}

pub(super) const fn agent_provider(provider: ProviderAuthProvider) -> AgentChatProvider {
    match provider {
        ProviderAuthProvider::Claude => AgentChatProvider::Claude,
        ProviderAuthProvider::Codex => AgentChatProvider::Codex,
    }
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_secs())
}

#[cfg(test)]
#[path = "provider_auth_probe_tests.rs"]
mod tests;
