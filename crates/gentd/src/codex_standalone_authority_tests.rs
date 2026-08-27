use std::fs;

use gent_drivers::process::SystemLauncher;
use gent_runtime::catalog::RuntimeCapabilityProfile;
use gent_types::HostEpoch;

use super::{StandaloneCodexConfig, StandaloneCodexError, compose_standalone_codex};
use crate::{CompatibilityAssessment, runtime_facade::DaemonCompositionState};

#[test]
fn captures_an_explicit_codex_binary_without_release_material() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("codex");
    fs::write(&executable, "standalone Codex executable").unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();

    let mut host = compose_standalone_codex(
        state.ledger().clone(),
        state.coordinator().clone(),
        &StandaloneCodexConfig {
            data_dir: tempfile::tempdir().unwrap().keep(),
            coordinator_id: "gentd-standalone".into(),
            host_epoch: HostEpoch(1),
            executable,
            mcp_servers: None,
            mcp_config: None,
        },
        SystemLauncher::new(64 * 1024),
    )
    .unwrap();

    assert!(host.wake().is_ok());
}

#[test]
fn rejects_missing_local_executable_before_host_composition() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let result = compose_standalone_codex(
        state.ledger().clone(),
        state.coordinator().clone(),
        &StandaloneCodexConfig {
            data_dir: tempfile::tempdir().unwrap().keep(),
            coordinator_id: "gentd-standalone".into(),
            host_epoch: HostEpoch(1),
            executable: directory.path().join("missing-codex"),
            mcp_servers: None,
            mcp_config: None,
        },
        SystemLauncher::new(64 * 1024),
    );
    assert!(matches!(result, Err(StandaloneCodexError::LocalLock(_))));
}
