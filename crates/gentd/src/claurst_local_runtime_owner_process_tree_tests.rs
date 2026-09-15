use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Command,
    sync::OnceLock,
    time::{Duration, Instant},
};

use super::super::{LocalRuntimeLauncher, LocalRuntimeProcess, SystemLocalRuntimeLauncher};
use crate::claurst_local_runtime::LocalProcessLaunch;

fn ledger() -> &'static PathBuf {
    static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
    DIRECTORY.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap().keep().join("process-groups");
        std::mem::forget(
            gent_drivers::process::groups::install(&directory)
                .unwrap()
                .0,
        );
        directory
    })
}

fn alive(pid: &str) -> bool {
    let output = Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .unwrap();
    let state = String::from_utf8(output.stdout).unwrap();
    !state.trim().is_empty() && !state.trim().starts_with('Z')
}

#[test]
fn a_llama_server_is_recorded_while_it_runs_and_its_tree_is_stopped_and_forgotten_on_shutdown() {
    let ledger = ledger();
    let directory = tempfile::tempdir().unwrap();
    let pids = directory.path().join("pids");
    let mut process = SystemLocalRuntimeLauncher
        .launch(&LocalProcessLaunch {
            executable: "/bin/sh".into(),
            arguments: vec![
                "-c".into(),
                format!(
                    "echo $$ > {0}.tmp; sleep 60 & echo $! >> {0}.tmp; mv {0}.tmp {0}; while :; do sleep 1; done",
                    pids.display()
                ),
            ],
            working_directory: None,
            environment: BTreeMap::new(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pids.exists() {
        assert!(
            Instant::now() < deadline,
            "the fake llama-server did not start"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let recorded = std::fs::read_to_string(&pids).unwrap();
    let (leader, server) = recorded.trim().split_once('\n').unwrap();
    assert!(ledger.join(leader).exists());

    process.shutdown().unwrap();

    assert!(!ledger.join(leader).exists());
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(server) {
        assert!(
            Instant::now() < deadline,
            "the llama-server tree survived shutdown"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
