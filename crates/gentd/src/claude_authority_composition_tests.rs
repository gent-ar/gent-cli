use std::path::PathBuf;

use gent_drivers::ReadOnlyHostLauncher;
use gent_runtime::catalog::RuntimeCapabilityProfile;
use gent_types::HostEpoch;

use super::{
    PrivateClaudeAuthorityConfig, PrivateClaudeAuthorityError, compose_private_claude_authority,
    validate,
};
use crate::CompatibilityAssessment;
use crate::runtime_facade::DaemonCompositionState;

fn config() -> PrivateClaudeAuthorityConfig {
    PrivateClaudeAuthorityConfig {
        evidence_record: PathBuf::from("/does-not-exist/claude-evidence.json"),
        trusted_keys: vec![
            "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ],
        coordinator_id: "private-claude-host".into(),
        host_epoch: HostEpoch(1),
        now_unix_seconds: 1,
    }
}

#[test]
fn private_claude_config_rejects_blank_or_unbounded_coordinator_before_preflight() {
    let mut blank = config();
    blank.coordinator_id = " ".into();
    assert!(matches!(
        validate(&blank),
        Err(PrivateClaudeAuthorityError::InvalidCoordinator)
    ));
    let mut oversized = config();
    oversized.coordinator_id = "x".repeat(257);
    assert!(matches!(
        validate(&oversized),
        Err(PrivateClaudeAuthorityError::InvalidCoordinator)
    ));
}

#[test]
fn missing_private_evidence_fails_before_a_claude_host_or_private_prefix_is_constructed() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(matches!(
        compose_private_claude_authority(&state, config(), ReadOnlyHostLauncher::new(1)),
        Err(PrivateClaudeAuthorityError::Preflight(_))
    ));
    assert!(
        !state
            .data_dir()
            .join("providers")
            .join("npm-global")
            .exists()
    );
}
