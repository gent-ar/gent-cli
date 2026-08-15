use std::path::PathBuf;

#[cfg(unix)]
use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeSignal};
use gent_drivers::{
    LaunchIntent, ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError, SystemLauncher,
};

fn shell_launch(script: &str) -> ProviderLaunch {
    ProviderLaunch {
        provider: "claude".into(),
        executable: PathBuf::from("/bin/sh"),
        arguments: vec!["-c".into(), script.into()],
        intent: LaunchIntent::Start,
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

#[test]
fn public_launcher_refuses_private_provider_names() {
    let mut launch = shell_launch("exit 0");
    launch.provider = "claurst".into();

    assert!(matches!(
        SystemLauncher::new(1).launch(&launch),
        Err(SupervisorError::UnsupportedProvider(provider)) if provider == "claurst"
    ));
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
