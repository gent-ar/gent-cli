use std::fs;

use gent_runtime::catalog::RuntimeCapabilityProfile;

use super::{
    LocalModelDownloadStart, StandaloneAuthorityConfig, StandaloneClaurstModelStatus,
    StandaloneClaurstModels, compose_standalone_authority,
};
use crate::{CompatibilityAssessment, runtime_facade::DaemonCompositionState};

#[test]
fn builds_both_local_provider_hosts_and_a_demand_driven_router() {
    let directory = tempfile::tempdir().unwrap();
    let claude = directory.path().join("claude");
    let codex = directory.path().join("codex");
    fs::write(&claude, "Claude executable").unwrap();
    fs::write(&codex, "Codex executable").unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();

    let runtime = compose_standalone_authority(
        &state,
        &StandaloneAuthorityConfig {
            data_dir: directory.path().into(),
            claude_executable: Some(claude),
            codex_executable: Some(codex),
            mcp_config: None,
        },
    )
    .unwrap();

    assert!(!runtime.drive_once().unwrap());
    assert!(matches!(
        runtime.claurst_models().assess("qwen3-1-7b-q4-k-m").unwrap(),
        StandaloneClaurstModelStatus::DownloadRequired { downloaded_bytes: 0, plan }
            if plan.model_id == "qwen3-1-7b-q4-k-m"
    ));
}

#[test]
fn starts_without_node_or_provider_executables_until_a_public_provider_is_selected() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let runtime = compose_standalone_authority(
        &state,
        &StandaloneAuthorityConfig {
            data_dir: directory.path().into(),
            claude_executable: None,
            codex_executable: None,
            mcp_config: None,
        },
    )
    .unwrap();
    assert!(!runtime.drive_once().unwrap());
    assert!(
        !directory
            .path()
            .join("providers/npm-global/bin/claude")
            .exists()
    );
    assert!(
        !directory
            .path()
            .join("providers/npm-global/bin/codex")
            .exists()
    );
}

#[test]
fn defers_explicit_provider_path_validation_until_its_provider_is_selected() {
    let directory = tempfile::tempdir().unwrap();
    let claude = directory.path().join("claude");
    fs::write(&claude, "Claude executable").unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();

    let runtime = compose_standalone_authority(
        &state,
        &StandaloneAuthorityConfig {
            data_dir: directory.path().into(),
            claude_executable: Some(claude),
            codex_executable: Some(directory.path().join("missing-codex")),
            mcp_config: None,
        },
    )
    .unwrap();
    assert!(!runtime.drive_once().unwrap());
}

#[test]
fn selected_claurst_model_claims_one_daemon_owned_download_until_completion() {
    let directory = tempfile::tempdir().unwrap();
    let models = StandaloneClaurstModels::from_data_dir(directory.path()).unwrap();
    assert!(matches!(
        models.begin_download("qwen3-1-7b-q4-k-m").unwrap(),
        LocalModelDownloadStart::Download { .. }
    ));
    assert!(models.download_active("qwen3-1-7b-q4-k-m"));
    assert!(models.begin_download("qwen3-1-7b-q4-k-m").is_err());
    models.finish_download("qwen3-1-7b-q4-k-m");
    assert!(!models.download_active("qwen3-1-7b-q4-k-m"));
    assert!(matches!(
        models.begin_download("qwen3-1-7b-q4-k-m").unwrap(),
        LocalModelDownloadStart::Download { .. }
    ));
}
