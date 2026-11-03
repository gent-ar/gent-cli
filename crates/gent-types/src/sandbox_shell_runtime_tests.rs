use crate::{
    RunVersionLock, SandboxLaunchProfile, SandboxNetworkPolicy, SandboxResourceGrant,
    SandboxResourceLimits, SandboxShellRuntime,
};

#[test]
fn sandbox_shell_requires_a_pinned_bash_and_readonly_exact_file_or_ancestor() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let workspace = root.join("workspace");
    let runtime = root.join("runtime");
    let image = runtime.join("bash.exe");
    let lock = RunVersionLock {
        provider: "git-bash".into(),
        canonical_path: image.to_string_lossy().into_owned(),
        file_identity: "test-shell".into(),
        digest_sha256: "a".repeat(64),
        version: "5.2.15".into(),
        compatibility_entry: "test-shell".into(),
    };
    let shell = SandboxShellRuntime::new(lock.clone()).unwrap();
    let profile = SandboxLaunchProfile::new(
        &workspace,
        std::slice::from_ref(&workspace),
        &[],
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 4,
            max_memory_bytes: 100_000,
            max_cpu_time_ms: 1_000,
        },
    )
    .unwrap();
    assert!(profile.clone().with_shell_runtime(shell.clone()).is_err());
    let exact = profile
        .clone()
        .with_resources(vec![SandboxResourceGrant::RuntimeReadOnly(image.clone())])
        .unwrap()
        .with_shell_runtime(shell.clone())
        .unwrap();
    assert!(exact.clone().with_resources(vec![]).is_err());
    assert!(
        exact
            .clone()
            .with_resources(vec![SandboxResourceGrant::RuntimeReadOnly(
                runtime.join("msys-2.0.dll")
            )])
            .is_err()
    );
    for grant in [
        SandboxResourceGrant::RuntimeReadOnly(runtime.join("other.exe")),
        SandboxResourceGrant::PrivateStateReadWrite(image.clone()),
    ] {
        assert!(
            profile
                .clone()
                .with_resources(vec![grant])
                .unwrap()
                .with_shell_runtime(shell.clone())
                .is_err()
        );
    }
    assert!(
        profile
            .clone()
            .with_resources(vec![SandboxResourceGrant::PrivateStateReadWrite(
                runtime.clone()
            )])
            .unwrap()
            .with_shell_runtime(shell.clone())
            .is_err()
    );
    let bound = profile
        .with_resources(vec![SandboxResourceGrant::RuntimeReadOnly(runtime.clone())])
        .unwrap()
        .with_shell_runtime(shell)
        .unwrap();
    assert!(bound.clone().with_resources(vec![]).is_err());
    let mut changed = lock.clone();
    changed.digest_sha256 = "b".repeat(64);
    let changed = bound
        .clone()
        .with_shell_runtime(SandboxShellRuntime::new(changed).unwrap())
        .unwrap();
    assert_ne!(bound.digest_sha256(), changed.digest_sha256());
    assert_ne!(
        exact.digest_sha256(),
        exact
            .clone()
            .with_shell_runtime(changed.shell_runtime().unwrap().clone())
            .unwrap()
            .digest_sha256()
    );
    let mut invalid = lock;
    invalid.provider = "provider".into();
    assert!(SandboxShellRuntime::new(invalid).is_err());
}

#[test]
fn command_boundary_rejects_every_broad_or_gateway_launch_without_brokered_egress() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let lock = |provider: &str, path: &std::path::Path| RunVersionLock {
        provider: provider.into(),
        canonical_path: path.to_string_lossy().into_owned(),
        file_identity: "fixture".into(),
        digest_sha256: "a".repeat(64),
        version: "fixture".into(),
        compatibility_entry: "fixture".into(),
    };
    let bash = root.join("bash.exe");
    let powershell = root.join("powershell/pwsh.exe");
    let launcher = root.join("powershell-launcher/pwsh.exe");
    let shell = SandboxShellRuntime::new(lock("git-bash", &bash))
        .unwrap()
        .with_powershell(lock("powershell7", &powershell))
        .unwrap();
    assert!(shell.clone().with_brokered_command_egress().is_err());
    let unbrokered = shell
        .with_powershell_launcher(lock("powershell7-launcher", &launcher))
        .unwrap();
    let brokered = unbrokered.clone().with_brokered_command_egress().unwrap();
    let digest = "c".repeat(64);
    let gateway = crate::SandboxProviderGateway::new(
        crate::SandboxGatewayApi::Claude,
        "127.0.0.1:43210".parse().unwrap(),
        "d".repeat(64),
        digest.clone(),
    )
    .unwrap();
    let template = SandboxLaunchProfile::new(
        &root.join("workspace"),
        &[root.join("workspace")],
        &[],
        vec![],
        SandboxNetworkPolicy::ReviewedEgress {
            policy_digest_sha256: digest,
        },
        SandboxResourceLimits {
            max_processes: 4,
            max_memory_bytes: 100_000,
            max_cpu_time_ms: 1_000,
        },
    )
    .unwrap()
    .with_resources(vec![
        SandboxResourceGrant::RuntimeReadOnly(bash),
        SandboxResourceGrant::RuntimeReadOnly(powershell),
        SandboxResourceGrant::RuntimeReadOnly(launcher),
    ])
    .unwrap();
    assert!(template.require_command_boundary(true).is_ok());
    let bound = template.clone().with_gateway(gateway).unwrap();
    assert!(bound.require_command_boundary(true).is_err());
    assert!(bound.require_command_boundary(false).is_err());
    let mut with_mode = template.clone();
    for mode in [
        crate::PermissionMode::Autonomous,
        crate::PermissionMode::Bypass,
    ] {
        with_mode = with_mode.with_permission_binding(
            crate::SandboxPermissionBinding::new(
                "conversation".into(),
                "run".into(),
                crate::PolicyRecord {
                    policy_id: "policy".into(),
                    workspace_id: "workspace".into(),
                    scope: crate::PolicyScope::ProviderPermissions,
                    revision: 1,
                    mode,
                    containment: crate::Containment::Sandboxed,
                    allowed_tools: vec![],
                    allowed_categories: vec![],
                },
                crate::HostEpoch(1),
            )
            .unwrap(),
        );
        assert!(with_mode.require_command_boundary(true).is_err());
        assert!(with_mode.require_command_boundary(false).is_err());
        assert!(
            with_mode
                .clone()
                .with_shell_runtime(brokered.clone())
                .unwrap()
                .require_command_boundary(true)
                .is_err()
        );
    }
    let without_marker = bound.clone().with_shell_runtime(unbrokered).unwrap();
    assert!(without_marker.require_command_boundary(true).is_err());
    let complete = bound.with_shell_runtime(brokered).unwrap();
    assert!(complete.require_command_boundary(true).is_ok());
    assert_ne!(complete.digest_sha256(), without_marker.digest_sha256());
}

#[test]
fn only_an_unregistered_worker_with_no_network_or_shell_skips_the_command_broker() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let binding = crate::SandboxPermissionBinding::new(
        "conversation".into(),
        "run".into(),
        crate::PolicyRecord {
            policy_id: "policy".into(),
            workspace_id: "workspace".into(),
            scope: crate::PolicyScope::ProviderPermissions,
            revision: 1,
            mode: crate::PermissionMode::Bypass,
            containment: crate::Containment::Sandboxed,
            allowed_tools: vec![],
            allowed_categories: vec![],
        },
        crate::HostEpoch(1),
    )
    .unwrap();
    let bash = root.join("bash.exe");
    let isolated = SandboxLaunchProfile::new(
        &root.join("workspace"),
        &[root.join("workspace")],
        &[],
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 2,
            max_memory_bytes: 100_000,
            max_cpu_time_ms: 1_000,
        },
    )
    .unwrap()
    .with_resources(vec![SandboxResourceGrant::RuntimeReadOnly(bash.clone())])
    .unwrap()
    .with_permission_binding(binding);
    assert!(isolated.require_command_boundary(false).is_ok());
    assert!(isolated.require_command_boundary(true).is_err());
    let uncapped = isolated
        .clone()
        .with_cpu_semantics(crate::SandboxCpuSemantics::Uncapped)
        .unwrap();
    assert!(uncapped.require_command_boundary(false).is_ok());
    assert!(uncapped.require_command_boundary(true).is_err());
    assert_ne!(uncapped.digest_sha256(), isolated.digest_sha256());
    let with_shell = isolated
        .with_shell_runtime(
            SandboxShellRuntime::new(RunVersionLock {
                provider: "git-bash".into(),
                canonical_path: bash.to_string_lossy().into_owned(),
                file_identity: "fixture".into(),
                digest_sha256: "a".repeat(64),
                version: "fixture".into(),
                compatibility_entry: "fixture".into(),
            })
            .unwrap(),
        )
        .unwrap();
    assert!(with_shell.require_command_boundary(false).is_err());
}

#[test]
fn powershell_companion_is_pinned_covered_and_part_of_profile_digest() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let bash = root.join("bash.exe");
    let powershell = root.join("powershell/pwsh.exe");
    let launcher = root.join("powershell-launcher/pwsh.exe");
    let lock = |provider: &str, path: &std::path::Path| RunVersionLock {
        provider: provider.into(),
        canonical_path: path.to_string_lossy().into_owned(),
        file_identity: "fixture".into(),
        digest_sha256: "a".repeat(64),
        version: "fixture".into(),
        compatibility_entry: "fixture".into(),
    };
    let shell = SandboxShellRuntime::new(lock("git-bash", &bash)).unwrap();
    let engine_only = shell
        .clone()
        .with_powershell(lock("powershell7", &powershell))
        .unwrap();
    let companion = engine_only
        .clone()
        .with_powershell_launcher(lock("powershell7-launcher", &launcher))
        .unwrap();
    assert!(
        shell
            .clone()
            .with_powershell_launcher(lock("powershell7-launcher", &launcher))
            .is_err()
    );
    assert!(
        engine_only
            .clone()
            .with_powershell_launcher(lock("powershell7", &launcher))
            .is_err()
    );
    assert!(
        engine_only
            .clone()
            .with_powershell_launcher(lock(
                "powershell7-launcher",
                &root.join("elsewhere/pwsh.exe")
            ))
            .is_err()
    );
    assert!(
        shell
            .clone()
            .with_powershell(lock("git-bash", &powershell))
            .is_err()
    );
    assert!(
        shell
            .clone()
            .with_powershell(lock("powershell7", &bash))
            .is_err()
    );
    let profile = SandboxLaunchProfile::new(
        &root.join("workspace"),
        &[root.join("workspace")],
        &[],
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 4,
            max_memory_bytes: 100_000,
            max_cpu_time_ms: 1_000,
        },
    )
    .unwrap()
    .with_resources(vec![SandboxResourceGrant::RuntimeReadOnly(bash)])
    .unwrap();
    assert!(
        profile
            .clone()
            .with_shell_runtime(companion.clone())
            .is_err()
    );
    let profile = profile
        .with_resources(vec![
            SandboxResourceGrant::RuntimeReadOnly(root.join("bash.exe")),
            SandboxResourceGrant::RuntimeReadOnly(powershell.clone()),
            SandboxResourceGrant::RuntimeReadOnly(launcher.clone()),
        ])
        .unwrap();
    let original = profile.clone().with_shell_runtime(shell).unwrap();
    assert!(profile.clone().with_shell_runtime(engine_only).is_err());
    let bound = profile.with_shell_runtime(companion.clone()).unwrap();
    assert_ne!(original.digest_sha256(), bound.digest_sha256());
    let mut changed = companion.powershell_lock().unwrap().clone();
    changed.digest_sha256 = "b".repeat(64);
    let changed = bound
        .clone()
        .with_shell_runtime(
            companion
                .clone()
                .with_powershell(changed)
                .unwrap()
                .with_powershell_launcher(lock("powershell7-launcher", &launcher))
                .unwrap(),
        )
        .unwrap();
    assert_ne!(bound.digest_sha256(), changed.digest_sha256());
    let mut launcher_changed = companion.powershell_launcher_lock().unwrap().clone();
    launcher_changed.digest_sha256 = "c".repeat(64);
    let changed = bound
        .clone()
        .with_shell_runtime(
            companion
                .with_powershell_launcher(launcher_changed)
                .unwrap(),
        )
        .unwrap();
    assert_ne!(bound.digest_sha256(), changed.digest_sha256());
    assert!(
        bound
            .with_resources(vec![
                SandboxResourceGrant::RuntimeReadOnly(root.join("bash.exe")),
                SandboxResourceGrant::PrivateStateReadWrite(powershell)
            ])
            .is_err()
    );
}
