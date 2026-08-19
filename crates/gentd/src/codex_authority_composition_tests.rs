use std::path::PathBuf;

use gent_drivers::ReadOnlyHostLauncher;
use gent_types::HostEpoch;

use super::{
    PrivateCodexAuthorityConfig, PrivateCodexAuthorityError, compose_private_codex_authority,
    validate,
};
use crate::CompatibilityAssessment;
use crate::runtime_facade::DaemonCompositionState;
use gent_runtime::catalog::RuntimeCapabilityProfile;

fn config() -> PrivateCodexAuthorityConfig {
    PrivateCodexAuthorityConfig {
        evidence_record: PathBuf::from("/does-not-exist/codex-evidence.json"),
        trusted_keys: vec![
            "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ],
        coordinator_id: "private-codex-host".into(),
        host_epoch: HostEpoch(1),
        now_unix_seconds: 1,
    }
}

#[test]
fn private_codex_config_rejects_blank_or_unbounded_coordinator_before_preflight() {
    let mut blank = config();
    blank.coordinator_id = " ".into();
    assert!(matches!(
        validate(&blank),
        Err(PrivateCodexAuthorityError::InvalidCoordinator)
    ));
    let mut oversized = config();
    oversized.coordinator_id = "x".repeat(257);
    assert!(matches!(
        validate(&oversized),
        Err(PrivateCodexAuthorityError::InvalidCoordinator)
    ));
}

#[test]
fn missing_private_evidence_fails_before_a_codex_host_is_constructed() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(matches!(
        compose_private_codex_authority(&state, &config(), ReadOnlyHostLauncher::new(1)),
        Err(PrivateCodexAuthorityError::Preflight(_))
    ));
    assert!(
        !state
            .data_dir()
            .join("providers")
            .join("npm-global")
            .exists()
    );
}
