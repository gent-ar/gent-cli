use std::path::PathBuf;

use gent_ports::{SandboxedProviderPreflight, SandboxedProviderPreflightError};
use gent_types::{
    HostEpoch, RunVersionLock, SandboxBackendId, SandboxEnforcement, SandboxLaunchAttestation,
    SandboxLaunchProfile, SandboxNetworkPolicy, SandboxResourceLimits, SandboxedLaunchRequest,
};

use super::{
    PrivateCodexAuthorityConfig, PrivateCodexAuthorityError, compose_private_codex_authority,
    validate,
};
use crate::CompatibilityAssessment;
use crate::runtime_facade::DaemonCompositionState;
use gent_runtime::catalog::declared_capabilities;

fn config() -> PrivateCodexAuthorityConfig {
    PrivateCodexAuthorityConfig {
        evidence_record: PathBuf::from("/does-not-exist/codex-evidence.json"),
        trusted_keys: vec![
            "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ],
        coordinator_id: "private-codex-host".into(),
        working_directory: None,
        host_epoch: HostEpoch(1),
        now_unix_seconds: 1,
        sandbox_request: sandbox_request(),
    }
}

fn sandbox_request() -> SandboxedLaunchRequest {
    SandboxedLaunchRequest {
        lock: RunVersionLock {
            provider: "codex".into(),
            canonical_path: "/private/gent/codex".into(),
            file_identity: "1:2".into(),
            digest_sha256: "a".repeat(64),
            version: "0.147.0".into(),
            compatibility_entry: "codex-0.147.0".into(),
        },
        profile: SandboxLaunchProfile::new(
            std::path::Path::new("/private/gent/workspace"),
            &[PathBuf::from("/private/gent/workspace")],
            &[],
            vec!["TERM".into()],
            SandboxNetworkPolicy::Disabled,
            SandboxResourceLimits {
                max_processes: 4,
                max_memory_bytes: 1_000_000,
                max_cpu_time_ms: 60_000,
            },
        )
        .unwrap(),
    }
}

#[derive(Clone, Copy)]
enum SandboxResult {
    Unavailable,
    ChangedLock,
    Attested,
}

struct Sandbox(SandboxResult);

impl SandboxedProviderPreflight for Sandbox {
    fn preflight(
        &self,
        request: &SandboxedLaunchRequest,
    ) -> Result<SandboxLaunchAttestation, SandboxedProviderPreflightError> {
        match self.0 {
            SandboxResult::Unavailable => Err(SandboxedProviderPreflightError::Unavailable),
            SandboxResult::ChangedLock => Err(SandboxedProviderPreflightError::LockChanged),
            SandboxResult::Attested => request
                .attest_after_lock_recheck(
                    &request.lock,
                    SandboxBackendId::new("test-native-sandbox".into()).unwrap(),
                    SandboxEnforcement::Enforced,
                )
                .map_err(|_| SandboxedProviderPreflightError::AttestationRejected),
        }
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
    let mut wrong_provider = config();
    wrong_provider.sandbox_request.lock.provider = "claude".into();
    assert!(matches!(
        validate(&wrong_provider),
        Err(PrivateCodexAuthorityError::InvalidSandboxRequest)
    ));
}

#[test]
fn missing_private_evidence_fails_before_a_codex_host_is_constructed() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &declared_capabilities(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(matches!(
        compose_private_codex_authority(&state, &config(), Sandbox(SandboxResult::Attested)),
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

#[test]
fn unavailable_sandbox_prevents_evidence_loading_or_host_construction() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &declared_capabilities(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(matches!(
        compose_private_codex_authority(&state, &config(), Sandbox(SandboxResult::Unavailable)),
        Err(PrivateCodexAuthorityError::Sandbox(
            SandboxedProviderPreflightError::Unavailable
        ))
    ));
    assert!(!state.data_dir().join("providers").exists());
}

#[test]
fn changed_lock_prevents_evidence_loading_or_host_construction() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &declared_capabilities(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    assert!(matches!(
        compose_private_codex_authority(&state, &config(), Sandbox(SandboxResult::ChangedLock)),
        Err(PrivateCodexAuthorityError::Sandbox(
            SandboxedProviderPreflightError::LockChanged
        ))
    ));
    assert!(!state.data_dir().join("providers").exists());
}
