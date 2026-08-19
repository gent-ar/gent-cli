use std::path::PathBuf;

use gent_runtime::catalog::RuntimeCapabilityProfile;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, HostEpoch,
};

use super::{OrdinaryAuthorityConfig, OrdinaryAuthorityError, compose_ordinary_authority};
use crate::CompatibilityAssessment;
use crate::claude_authority_composition::PrivateClaudeAuthorityConfig;
use crate::codex_authority_composition::PrivateCodexAuthorityConfig;
use crate::runtime_facade::DaemonCompositionState;

fn config() -> OrdinaryAuthorityConfig {
    OrdinaryAuthorityConfig {
        codex: PrivateCodexAuthorityConfig {
            evidence_record: PathBuf::from("/does-not-exist/codex-evidence.json"),
            trusted_keys: vec![
                "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            ],
            coordinator_id: "ordinary-owner".into(),
            host_epoch: HostEpoch(1),
            now_unix_seconds: 1,
        },
        claude: PrivateClaudeAuthorityConfig {
            evidence_record: PathBuf::from("/does-not-exist/claude-evidence.json"),
            trusted_keys: vec![
                "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            ],
            coordinator_id: "ordinary-owner".into(),
            host_epoch: HostEpoch(1),
            now_unix_seconds: 1,
        },
        selections: vec![AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Ask,
        }],
    }
}

#[test]
fn a_failed_first_preflight_constructs_no_private_provider_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();

    assert!(matches!(
        compose_ordinary_authority(&state, config()),
        Err(OrdinaryAuthorityError::CodexPreflight(_))
    ));
    assert!(
        !state
            .data_dir()
            .join("providers")
            .join("npm-global")
            .exists()
    );
}

#[test]
fn mismatched_owners_fail_before_reading_evidence_or_constructing_a_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let mut input = config();
    input.claude.coordinator_id = "different-owner".into();

    assert!(matches!(
        compose_ordinary_authority(&state, input),
        Err(OrdinaryAuthorityError::CoordinatorMismatch)
    ));
    assert!(!state.data_dir().join("providers").exists());
}

#[test]
fn mismatched_epochs_fail_before_reading_evidence_or_constructing_a_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let mut input = config();
    input.claude.host_epoch = HostEpoch(2);

    assert!(matches!(
        compose_ordinary_authority(&state, input),
        Err(OrdinaryAuthorityError::HostEpochMismatch)
    ));
    assert!(!state.data_dir().join("providers").exists());
}

#[test]
fn invalid_selection_sets_fail_before_reading_evidence_or_constructing_a_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let mut empty = config();
    empty.selections.clear();
    assert!(matches!(
        compose_ordinary_authority(&state, empty),
        Err(OrdinaryAuthorityError::MissingSelections)
    ));

    let mut duplicate = config();
    duplicate.selections.push(duplicate.selections[0].clone());
    assert!(matches!(
        compose_ordinary_authority(&state, duplicate),
        Err(OrdinaryAuthorityError::DuplicateSelection)
    ));

    let mut unsupported = config();
    unsupported.selections[0].provider = AgentChatProvider::Claurst;
    assert!(matches!(
        compose_ordinary_authority(&state, unsupported),
        Err(OrdinaryAuthorityError::UnsupportedSelection)
    ));
    assert!(!state.data_dir().join("providers").exists());
}

#[test]
fn agent_mode_fails_before_reading_evidence_or_constructing_a_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let mut input = config();
    input.selections[0].mode = AgentChatMode::Agent;

    assert!(matches!(
        compose_ordinary_authority(&state, input),
        Err(OrdinaryAuthorityError::UnsupportedSelection)
    ));
    assert!(!state.data_dir().join("providers").exists());
}
