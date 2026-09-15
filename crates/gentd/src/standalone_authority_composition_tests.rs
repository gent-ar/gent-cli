use std::fs;

use gent_runtime::catalog::RuntimeCapabilityProfile;

use super::{
    StandaloneAuthorityConfig, claurst_models::StandaloneClaurstModelStatus,
    compose_standalone_authority,
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
            executables: crate::provider_executables::ProviderExecutables::standalone(
                Some(claude),
                Some(codex),
                directory.path(),
                state.ledger().clone(),
                None,
            ),
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
            executables: crate::provider_executables::ProviderExecutables::standalone(
                None,
                None,
                directory.path(),
                state.ledger().clone(),
                None,
            ),
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
            executables: crate::provider_executables::ProviderExecutables::standalone(
                Some(claude),
                Some(directory.path().join("missing-codex")),
                directory.path(),
                state.ledger().clone(),
                None,
            ),
            mcp_config: None,
        },
    )
    .unwrap();
    assert!(!runtime.drive_once().unwrap());
}

#[tokio::test]
async fn selected_claurst_model_claims_one_daemon_owned_download_until_released() {
    let directory = tempfile::tempdir().unwrap();
    let models = crate::local_model_jobs::tests::models(directory.path()).with_download_transport(
        std::sync::Arc::new(crate::local_model_jobs::tests::BlockingTransport),
    );
    let holder = crate::local_model_jobs::DownloadHolder::Prompt("prompt".into());
    assert!(matches!(
        models
            .downloads
            .start("qwen3-1-7b-q4-k-m", holder.clone())
            .unwrap(),
        crate::local_model_jobs::DownloadStart::Downloading(_)
    ));
    assert!(matches!(
        models.install_state("qwen3-1-7b-q4-k-m").unwrap(),
        gent_protocol::LocalModelInstallState::Downloading {
            downloaded_bytes: 0,
            total_bytes: 1_282_439_264
        }
    ));
    models.downloads.release("qwen3-1-7b-q4-k-m", &holder);
    assert_eq!(
        models.install_state("qwen3-1-7b-q4-k-m").unwrap(),
        gent_protocol::LocalModelInstallState::NotInstalled
    );
}

#[cfg(unix)]
#[path = "standalone_authority_installed_provider_tests.rs"]
mod installed_provider;
