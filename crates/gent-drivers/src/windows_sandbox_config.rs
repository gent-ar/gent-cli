use crate::{ProviderLaunch, SandboxedProviderLaunchError as Error};
use gent_types::{RunVersionLock, SandboxedLaunchRequest};
use std::path::Path;
use std::process::{Command, Stdio};

#[path = "windows_sandbox_bootstrap.rs"]
mod bootstrap;
pub(super) use bootstrap::write_gateway_bootstrap;

pub(super) fn command(
    request: &SandboxedLaunchRequest,
    launch: &ProviderLaunch,
    helper: &RunVersionLock,
    helper_root: &Path,
    run: &str,
) -> Result<Command, Error> {
    let mut command = gent_types::hidden_command(&helper.canonical_path);
    command.current_dir(helper_root);
    command
        .arg("run")
        .arg("--workspace")
        .arg(request.profile.workspace_root());
    let powershell = request
        .profile
        .shell_runtime()
        .and_then(|shell| shell.powershell_lock());
    let launcher = request
        .profile
        .shell_runtime()
        .and_then(|shell| shell.powershell_launcher_lock());
    if powershell.is_some() != launcher.is_some() {
        return Err(Error::ProfileRejected);
    }
    if powershell.is_some() {
        command.arg("--read-stdin").arg("1");
    } else {
        for root in request.profile.readable_roots() {
            command.arg("--read").arg(root);
        }
    }
    for root in request.profile.writable_roots() {
        command.arg("--write").arg(root);
    }
    for resource in request.profile.resources() {
        if powershell.is_some() && !resource.writable() {
            continue;
        }
        command
            .arg(if resource.writable() {
                "--write"
            } else {
                "--read"
            })
            .arg(resource.root());
    }
    for name in request.profile.inherited_environment() {
        if matches!(name.as_str(), "TMP" | "TMPDIR") {
            return Err(Error::ProfileRejected);
        }
        if let Some(value) = std::env::var_os(name) {
            let mut entry = std::ffi::OsString::from(name);
            entry.push("=");
            entry.push(value);
            command.arg("--env").arg(entry);
        }
    }
    if let Some(gateway) = request.profile.gateway() {
        command
            .arg("--allow-loopback-port")
            .arg(gateway.endpoint().port().to_string())
            .arg("--gateway-api")
            .arg(gateway.api().provider_id())
            .arg("--gateway-url")
            .arg(gateway.origin())
            .arg("--gateway-token-stdin")
            .arg("1")
            .arg("--peer-nonce")
            .arg(gateway.peer_nonce());
    }
    if let Some(state) = request.profile.private_state() {
        command.arg("--state-root").arg(state.root());
    }
    if let Some(shell) = request.profile.shell_runtime() {
        command
            .arg("--shell-exe")
            .arg(&shell.lock().canonical_path)
            .arg("--shell-sha256")
            .arg(&shell.lock().digest_sha256);
        if let Some(powershell) = shell.powershell_lock() {
            command
                .arg("--powershell-exe")
                .arg(&powershell.canonical_path)
                .arg("--powershell-sha256")
                .arg(&powershell.digest_sha256);
        }
        if let Some(launcher) = shell.powershell_launcher_lock() {
            command
                .arg("--powershell-launcher-exe")
                .arg(&launcher.canonical_path)
                .arg("--powershell-launcher-sha256")
                .arg(&launcher.digest_sha256);
        }
    }
    if request.profile.cpu_semantics() == gent_types::SandboxCpuSemantics::Uncapped {
        command.arg("--cpu-uncapped").arg("1");
    }
    let limits = request.profile.limits();
    command
        .arg("--exe")
        .arg(&request.lock.canonical_path)
        .arg("--sha256")
        .arg(&request.lock.digest_sha256)
        .arg("--profile")
        .arg(request.profile.digest_sha256())
        .arg("--run")
        .arg(run)
        .arg("--processes")
        .arg(limits.max_processes.to_string())
        .arg("--memory")
        .arg(limits.max_memory_bytes.to_string())
        .arg("--cpu-ms")
        .arg(limits.max_cpu_time_ms.to_string())
        .arg("--")
        .args(&launch.arguments)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gent_types::{
        SandboxGatewayApi, SandboxLaunchPolicy, SandboxNetworkPolicy, SandboxProviderGateway,
        SandboxResourceLimits, SandboxWorkspaceAccess,
    };
    #[test]
    fn gateway_secret_is_absent_from_all_helper_arguments_and_environment() {
        let secret = "c".repeat(64);
        let gateway = SandboxProviderGateway::new(
            SandboxGatewayApi::Codex,
            "127.0.0.1:54321".parse().unwrap(),
            secret.clone(),
            "a".repeat(64),
        )
        .unwrap();
        let limits = SandboxResourceLimits {
            max_processes: 1,
            max_memory_bytes: 1024,
            max_cpu_time_ms: 1000,
        };
        let workspace = gent_testkit::host_absolute_path("/workspace");
        let profile = SandboxLaunchPolicy::new(
            vec![],
            SandboxNetworkPolicy::ReviewedEgress {
                policy_digest_sha256: "a".repeat(64),
            },
            limits,
        )
        .unwrap()
        .profile_for_workspace(&workspace, SandboxWorkspaceAccess::ReadOnly)
        .unwrap()
        .with_gateway(gateway)
        .unwrap();
        let lock = RunVersionLock {
            provider: "codex".into(),
            canonical_path: "C:/qa/codex.exe".into(),
            file_identity: "1:2".into(),
            digest_sha256: "b".repeat(64),
            version: "1".into(),
            compatibility_entry: "fixture".into(),
        };
        let request = SandboxedLaunchRequest {
            lock: lock.clone(),
            profile,
        };
        let launch = ProviderLaunch {
            permission_binding: None,
            run_id: Some("run-1".into()),
            lock: lock.clone(),
            provider: "codex".into(),
            executable: lock.canonical_path.clone().into(),
            arguments: vec![],
            intent: crate::LaunchIntent::Start,
            workspace_root: Some(workspace),
            workspace_access: SandboxWorkspaceAccess::ReadOnly,
        };
        let cmd = command(&request, &launch, &lock, Path::new("C:/qa"), "run-1").unwrap();
        let args = cmd
            .get_args()
            .map(|v| v.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.windows(2).any(|v| v == ["--gateway-token-stdin", "1"]));
        assert!(
            !args
                .iter()
                .any(|v| v.contains(&secret) || v == "--gateway-token")
        );
        assert!(
            !cmd.get_envs()
                .any(|(_, v)| v.is_some_and(|v| v.to_string_lossy().contains(&secret)))
        );
        let old_frame = bootstrap::encode(&request).unwrap();
        assert_eq!(old_frame.len(), 72);
        assert_eq!(&old_frame[..8], b"GENTGW01");
        let bash = gent_testkit::host_absolute_path("/runtime/bash.exe");
        let powershell = gent_testkit::host_absolute_path("/powershell/pwsh.exe");
        let ps_launcher = gent_testkit::host_absolute_path("/powershell-launcher/pwsh.exe");
        let companion = |provider: &str, path: &std::path::Path| RunVersionLock {
            provider: provider.into(),
            canonical_path: path.to_string_lossy().into_owned(),
            ..lock.clone()
        };
        let shell = gent_types::SandboxShellRuntime::new(companion("git-bash", &bash))
            .unwrap()
            .with_powershell(companion("powershell7", &powershell))
            .unwrap()
            .with_powershell_launcher(companion("powershell7-launcher", &ps_launcher))
            .unwrap();
        let profile = request
            .profile
            .clone()
            .with_resources(vec![
                gent_types::SandboxResourceGrant::RuntimeReadOnly(bash),
                gent_types::SandboxResourceGrant::RuntimeReadOnly(powershell),
                gent_types::SandboxResourceGrant::RuntimeReadOnly(ps_launcher),
            ])
            .unwrap()
            .with_shell_runtime(shell)
            .unwrap();
        let compact_request = SandboxedLaunchRequest {
            lock: lock.clone(),
            profile,
        };
        let compact = command(
            &compact_request,
            &launch,
            &lock,
            Path::new("C:/qa"),
            "run-1",
        )
        .unwrap();
        let arguments: Vec<_> = compact
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(arguments.windows(2).any(|a| a == ["--read-stdin", "1"]));
        assert!(arguments.contains(&"--powershell-sha256".into()));
        assert!(arguments.contains(&"--powershell-launcher-sha256".into()));
        assert!(!arguments.contains(&"--read".into()));
        assert!(!arguments.iter().any(|a| a.contains(&secret)));
        let frame = bootstrap::encode(&compact_request).unwrap();
        assert_eq!(&frame[..72], old_frame.as_slice());
        assert_eq!(&frame[72..80], b"GENTRO01");
        assert_eq!(u32::from_le_bytes(frame[80..84].try_into().unwrap()), 4);
    }
}
