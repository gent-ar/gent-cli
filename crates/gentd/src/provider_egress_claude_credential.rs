use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gent_types::RunVersionLock;
use serde_json::{Value, json};

use super::{
    CredentialProbePhase, EgressError,
    credential_capture::{CapturedProviderAuth, CredentialCapture},
    probe_process,
};

#[cfg(test)]
const VALIDATED_WINDOWS_CLAUDE: &str =
    "70052a17e06561a4563597f79570e81ccc0772474c3c6ac522aa5ccc71773013";

#[cfg(test)]
pub(super) fn resolve(
    lock: &RunVersionLock,
    probe_root: &Path,
) -> Result<CapturedProviderAuth, EgressError> {
    if !cfg!(windows) || lock.provider != "claude" || lock.digest_sha256 != VALIDATED_WINDOWS_CLAUDE
    {
        return Err(EgressError::Authentication);
    }
    resolve_reviewed(lock, probe_root)
}

#[cfg(windows)]
pub(super) fn resolve_managed(
    lock: &RunVersionLock,
    probe_root: &Path,
    resources: &crate::windows_provider_resources::ManagedProviderResources,
) -> Result<CapturedProviderAuth, EgressError> {
    if !resources.permits_claude_credentials(lock) {
        return Err(EgressError::Authentication);
    }
    resolve_reviewed(lock, probe_root)
}

#[path = "provider_egress_probe_directory.rs"]
mod probe_directory;
#[path = "provider_egress_claude_probe_options.rs"]
mod probe_options;
#[path = "provider_egress_probe_owner.rs"]
mod probe_owner;
use probe_options::{arguments, isolate_environment};

#[cfg(windows)]
fn resolve_reviewed(
    lock: &RunVersionLock,
    probe_root: &Path,
) -> Result<CapturedProviderAuth, EgressError> {
    resolve_using(
        lock,
        probe_root,
        probe_process::environment(),
        probe_process::run,
    )
}

pub(super) fn resolve_using(
    lock: &RunVersionLock,
    probe_root: &Path,
    environment: BTreeMap<String, String>,
    run: impl Fn(
        &RunVersionLock,
        &Path,
        &[String],
        &BTreeMap<String, String>,
        Duration,
    ) -> Result<probe_process::ProbeOutput, probe_process::ProbeFailure>,
) -> Result<CapturedProviderAuth, EgressError> {
    resolve_with_owner(
        lock,
        probe_root,
        environment,
        run,
        probe_owner::process_owner(),
    )
}

fn resolve_with_owner(
    lock: &RunVersionLock,
    probe_root: &Path,
    mut environment: BTreeMap<String, String>,
    run: impl Fn(
        &RunVersionLock,
        &Path,
        &[String],
        &BTreeMap<String, String>,
        Duration,
    ) -> Result<probe_process::ProbeOutput, probe_process::ProbeFailure>,
    owner: &probe_owner::Owner,
) -> Result<CapturedProviderAuth, EgressError> {
    let mut permit = owner.enter()?;
    let phase = std::cell::Cell::new(CredentialProbePhase::Preparation);
    let parent = std::fs::canonicalize(probe_root).map_err(|_| EgressError::Authentication)?;
    let mut directory = probe_directory::Directory::create(&parent)?;
    let root = directory.path().to_path_buf();
    let cwd = root.join("cwd");
    std::fs::create_dir(&cwd).map_err(|_| EgressError::Authentication)?;
    let mut unknown = None;
    let mut invoke = |args: &[String], env: &BTreeMap<String, String>, timeout| {
        directory.in_flight();
        match run(lock, &cwd, args, env, timeout) {
            Ok(output) => {
                directory.settled();
                Ok(output)
            }
            Err(failure) if failure.known => {
                directory.settled();
                Err(EgressError::Authentication)
            }
            Err(failure) => {
                unknown = Some(failure);
                Err(EgressError::Authentication)
            }
        }
    };
    let result = (|| {
        if let Ok(config) = std::env::var("CLAUDE_CONFIG_DIR") {
            environment.insert("CLAUDE_CONFIG_DIR".into(), config);
        }
        phase.set(CredentialProbePhase::StatusExecution);
        let status = invoke(
            &["auth".into(), "status".into(), "--json".into()],
            &environment,
            Duration::from_secs(15),
        )?;
        if !status.success || !status.stderr_empty {
            return Err(EgressError::Authentication);
        }
        phase.set(CredentialProbePhase::StatusProtocol);
        let status: Value =
            serde_json::from_slice(&status.stdout).map_err(|_| EgressError::Authentication)?;
        phase.set(CredentialProbePhase::LoginUnavailable);
        if status["authMethod"] != "claude.ai" || status["loggedIn"] != true {
            return Err(EgressError::Authentication);
        }
        phase.set(CredentialProbePhase::StatusProtocol);
        let config = status["configDirectory"]
            .as_str()
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or(EgressError::Authentication)?;
        let project = format!("gent-credential-probe-{}", uuid::Uuid::new_v4().simple());
        let transcript_namespace = config.join("projects").join(&project);
        phase.set(CredentialProbePhase::Transcript);
        if transcript_namespace.exists() {
            return Err(EgressError::Authentication);
        }
        environment.insert(
            "CLAUDE_CONFIG_DIR".into(),
            config.to_string_lossy().into_owned(),
        );
        environment.insert("CLAUDE_CODE_PROJECT_DIR_NAME".into(), project);
        isolate_environment(&mut environment, &root);
        phase.set(CredentialProbePhase::CaptureTransport);
        let capture = CredentialCapture::start()?;
        environment.insert("ANTHROPIC_BASE_URL".into(), capture.base_url.clone());
        let output = invoke(&arguments(), &environment, Duration::from_secs(45))?;
        let credential = capture.finish()?;
        phase.set(CredentialProbePhase::CaptureProtocol);
        if !output.success || !output.stderr_empty || !valid_output(&output.stdout) {
            return Err(EgressError::Authentication);
        }
        phase.set(CredentialProbePhase::Transcript);
        if has_transcript(&transcript_namespace)? {
            return Err(EgressError::Authentication);
        }
        Ok(credential)
    })();
    if let Some(failure) = unknown {
        permit.retain(directory, failure);
        return Err(EgressError::CredentialProbe {
            phase: CredentialProbePhase::ScopeUnknown,
            cleanup_failed: false,
            cleanup_unproven: false,
        });
    }
    let cleanup_failed = directory.cleanup().is_err();
    let mut cleanup_unproven = false;
    if cleanup_failed {
        let failure = directory.take_cleanup_failure();
        cleanup_unproven = !failure.known;
        permit.retain_primary(
            directory,
            failure,
            if result.is_err() {
                Some(phase.get())
            } else {
                None
            },
        );
    }
    match result {
        Ok(auth) if !cleanup_failed => Ok(auth),
        Ok(_) => Err(EgressError::CredentialProbe {
            phase: CredentialProbePhase::Cleanup,
            cleanup_failed: true,
            cleanup_unproven,
        }),
        Err(_) => Err(EgressError::CredentialProbe {
            phase: phase.get(),
            cleanup_failed,
            cleanup_unproven,
        }),
    }
}

fn valid_output(output: &[u8]) -> bool {
    let frames: Vec<Value> = output
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect();
    let Some(init) = frames
        .iter()
        .find(|frame| frame["type"] == "system" && frame["subtype"] == "init")
    else {
        return false;
    };
    ["tools", "mcp_servers", "plugins", "slash_commands"]
        .iter()
        .all(|field| init[*field].as_array().is_some_and(Vec::is_empty))
        && frames.iter().any(|frame| {
            frame["type"] == "result" && frame["result"] == "OK" && frame["is_error"] == false
        })
}

fn has_transcript(root: &Path) -> Result<bool, EgressError> {
    if !root.exists() {
        return Ok(false);
    }
    for entry in std::fs::read_dir(root).map_err(|_| EgressError::Authentication)? {
        let entry = entry.map_err(|_| EgressError::Authentication)?;
        let kind = entry.file_type().map_err(|_| EgressError::Authentication)?;
        if kind.is_symlink()
            || entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            || (kind.is_dir() && has_transcript(&entry.path())?)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "provider_egress_claude_probe_lifecycle_tests.rs"]
mod lifecycle_tests;
