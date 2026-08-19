use std::{fs, path::PathBuf};

use gent_runtime::catalog::RuntimeCapabilityProfile;
use gent_types::HostEpoch;

use super::{
    OrdinaryAuthorityConfig, OrdinaryAuthorityError, OrdinaryProviderAuthorityConfig,
    compose_ordinary_authority,
};
use crate::CompatibilityAssessment;
use crate::claude_authority_composition::PrivateClaudeAuthorityConfig;
use crate::codex_authority_composition::PrivateCodexAuthorityConfig;
use crate::node_runtime_lock::AppNodeRuntimeLock;
use crate::runtime_facade::DaemonCompositionState;

fn config(_: &std::path::Path) -> OrdinaryAuthorityConfig {
    OrdinaryAuthorityConfig {
        providers: vec![
            OrdinaryProviderAuthorityConfig::Codex(PrivateCodexAuthorityConfig {
                evidence_record: PathBuf::from("/does-not-exist/codex-evidence.json"),
                trusted_keys: vec![
                    "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                ],
                coordinator_id: "ordinary-owner".into(),
                host_epoch: HostEpoch(1),
                now_unix_seconds: 1,
            }),
            OrdinaryProviderAuthorityConfig::Claude(PrivateClaudeAuthorityConfig {
                evidence_record: PathBuf::from("/does-not-exist/claude-evidence.json"),
                trusted_keys: vec![
                    "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                ],
                coordinator_id: "ordinary-owner".into(),
                host_epoch: HostEpoch(1),
                now_unix_seconds: 1,
            }),
        ],
    }
}

fn app_node(root: &std::path::Path) -> AppNodeRuntimeLock {
    let bin = root.join("node/bin");
    fs::create_dir_all(&bin).unwrap();
    let node = bin.join("node");
    fs::write(&node, "node").unwrap();
    fs::write(bin.join("npm"), "npm").unwrap();
    let cli = root.join("node/lib/node_modules/npm/bin");
    fs::create_dir_all(&cli).unwrap();
    fs::write(cli.join("npm-cli.js"), "npm cli").unwrap();
    AppNodeRuntimeLock::capture(Some(node.into_os_string()), root).unwrap()
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
        compose_ordinary_authority(
            &state,
            config(directory.path()),
            &app_node(directory.path()),
        ),
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
    let mut input = config(directory.path());
    let OrdinaryProviderAuthorityConfig::Claude(claude) = &mut input.providers[1] else {
        unreachable!();
    };
    claude.coordinator_id = "different-owner".into();

    assert!(matches!(
        compose_ordinary_authority(&state, input, &app_node(directory.path())),
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
    let mut input = config(directory.path());
    let OrdinaryProviderAuthorityConfig::Claude(claude) = &mut input.providers[1] else {
        unreachable!();
    };
    claude.host_epoch = HostEpoch(2);

    assert!(matches!(
        compose_ordinary_authority(&state, input, &app_node(directory.path())),
        Err(OrdinaryAuthorityError::HostEpochMismatch)
    ));
    assert!(!state.data_dir().join("providers").exists());
}

#[test]
fn one_provider_preflight_does_not_require_another_provider_record() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &RuntimeCapabilityProfile::default(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let input = OrdinaryAuthorityConfig {
        providers: vec![
            config(directory.path())
                .providers
                .into_iter()
                .next()
                .unwrap(),
        ],
    };

    assert!(matches!(
        compose_ordinary_authority(&state, input, &app_node(directory.path())),
        Err(OrdinaryAuthorityError::CodexPreflight(_))
    ));
    assert!(!state.data_dir().join("providers").exists());
}
