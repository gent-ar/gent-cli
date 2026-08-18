use std::path::PathBuf;

use gent_ports::{SandboxedProviderPreflight, SandboxedProviderPreflightError};
use gent_runtime::catalog::declared_capabilities;
use gent_types::{
    HostEpoch, SandboxLaunchProfile, SandboxNetworkPolicy, SandboxResourceLimits,
    SandboxedLaunchRequest,
};

use super::{
    PrivateClaudeAuthorityConfig, PrivateClaudeAuthorityError, compose_private_claude_authority,
    validate,
};
use crate::CompatibilityAssessment;
use crate::runtime_facade::DaemonCompositionState;

#[derive(Debug)]
struct UnavailableSandbox;

impl SandboxedProviderPreflight for UnavailableSandbox {
    fn preflight(
        &self,
        _: &SandboxedLaunchRequest,
    ) -> Result<gent_types::SandboxLaunchAttestation, SandboxedProviderPreflightError> {
        Err(SandboxedProviderPreflightError::Unavailable)
    }
}

fn sandbox_profile() -> SandboxLaunchProfile {
    let workspace = PathBuf::from("/workspace");
    SandboxLaunchProfile::new(
        &workspace,
        std::slice::from_ref(&workspace),
        std::slice::from_ref(&workspace),
        vec!["LANG".into()],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 4,
            max_memory_bytes: 1,
            max_cpu_time_ms: 1,
        },
    )
    .unwrap()
}

fn config() -> PrivateClaudeAuthorityConfig<UnavailableSandbox> {
    PrivateClaudeAuthorityConfig {
        evidence_record: PathBuf::from("/does-not-exist/claude-evidence.json"),
        trusted_keys: vec![
            "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ],
        coordinator_id: "private-claude-host".into(),
        host_epoch: HostEpoch(1),
        now_unix_seconds: 1,
        sandbox_profile: sandbox_profile(),
        sandbox_preflight: UnavailableSandbox,
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
        &declared_capabilities(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(matches!(
        compose_private_claude_authority(&state, config()),
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
