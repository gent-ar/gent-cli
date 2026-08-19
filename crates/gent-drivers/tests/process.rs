#[cfg(unix)]
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeSignal};
use gent_drivers::{
    LaunchIntent, ProcessLauncher, ProviderLaunch, SupervisorError, SystemLauncher,
};

#[cfg(unix)]
use gent_drivers::ProviderProcess;

fn shell_launch(script: &str) -> ProviderLaunch {
    let lock =
        gent_drivers::lock::capture("claude", std::path::Path::new("/bin/sh"), "test", "test")
            .unwrap();
    ProviderLaunch {
        executable: lock.canonical_path.clone().into(),
        lock,
        provider: "claude".into(),
        arguments: vec!["-c".into(), script.into()],
        intent: LaunchIntent::Start,
        workspace_root: None,
        workspace_access: gent_types::SandboxWorkspaceAccess::ReadOnly,
    }
}

#[cfg(unix)]
#[test]
fn captures_each_stream_without_exceeding_its_limit() {
    let launcher = SystemLauncher::new(5);
    let process = launcher
        .launch(&shell_launch("printf abcdef; printf ghijkl >&2"))
        .unwrap();

    assert!(process.wait().unwrap().success());
    assert_eq!(process.output().stdout.bytes, b"abcde");
    assert_eq!(process.output().stdout.discarded_bytes, 1);
    assert_eq!(process.output().stderr.bytes, b"ghijk");
    assert_eq!(process.output().stderr.discarded_bytes, 1);
}

#[cfg(unix)]
#[test]
fn direct_wait_drains_a_full_stdout_delivery_queue() {
    let process = SystemLauncher::new(8)
        .launch(&shell_launch(
            "dd if=/dev/zero bs=4096 count=32 2>/dev/null",
        ))
        .unwrap();

    assert!(process.wait().unwrap().success());
    assert_eq!(process.output().stdout.bytes.len(), 8);
    assert_eq!(process.output().stdout.discarded_bytes, 131_064);
}

#[test]
fn public_launcher_refuses_private_provider_names() {
    let mut launch = shell_launch("exit 0");
    launch.provider = "claurst".into();

    assert!(matches!(
        SystemLauncher::new(1).launch(&launch),
        Err(SupervisorError::UnsupportedProvider(provider)) if provider == "claurst"
    ));
}

#[test]
fn system_launcher_rechecks_the_exact_launch_lock() {
    let mut launch = shell_launch("exit 0");
    launch.lock.digest_sha256 = "0".repeat(64);
    assert!(matches!(
        SystemLauncher::new(1).launch(&launch),
        Err(SupervisorError::Lock(_))
    ));
}

#[cfg(unix)]
#[test]
fn system_launcher_uses_the_durable_workspace_as_its_current_directory() {
    let workspace = tempfile::tempdir().unwrap();
    let mut launch = shell_launch("pwd");
    launch.workspace_root = Some(workspace.path().into());
    let process = SystemLauncher::new(4096).launch(&launch).unwrap();
    assert!(process.wait().unwrap().success());
    let actual = String::from_utf8(process.output().stdout.bytes).unwrap();
    assert_eq!(
        actual.trim(),
        workspace
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );
}

#[cfg(unix)]
#[test]
fn owned_process_writes_only_to_its_piped_standard_input() {
    let process = SystemLauncher::new(16)
        .launch(&shell_launch("IFS= read -r line; printf '%s' \"$line\""))
        .unwrap();

    process.write_frame(b"frame\n").unwrap();
    assert!(process.wait().unwrap().success());
    assert_eq!(process.output().stdout.bytes, b"frame");
}

#[cfg(unix)]
#[test]
fn kill_targets_the_provider_process_group() {
    let process = SystemLauncher::new(8)
        .launch(&shell_launch("while :; do sleep 1; done"))
        .unwrap();

    process.signal_tree(ProcessTreeSignal::Kill).unwrap();
    assert!(!process.wait().unwrap().success());
}
