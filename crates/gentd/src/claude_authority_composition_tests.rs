use std::path::PathBuf;

use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use gent_drivers::supervisor::{ProviderLaunch, ProviderProcess};
use gent_drivers::{SandboxedProviderLaunch, SandboxedProviderLaunchError};
use gent_runtime::catalog::declared_capabilities;
use gent_types::{
    HostEpoch, RunVersionLock, SandboxLaunchProfile, SandboxNetworkPolicy, SandboxResourceLimits,
    SandboxedLaunchRequest,
};

use super::{
    PrivateClaudeAuthorityConfig, PrivateClaudeAuthorityError, compose_private_claude_authority,
    validate,
};
use crate::CompatibilityAssessment;
use crate::runtime_facade::DaemonCompositionState;

#[derive(Debug)]
struct Sandbox;
#[derive(Debug)]
struct Process;
impl ProcessTreeControl for Process {
    fn signal_tree(&self, _: ProcessTreeSignal) -> Result<(), ProcessTreeError> {
        Ok(())
    }
}
impl ProviderProcess for Process {
    fn write_frame(&self, _: &[u8]) -> Result<(), ProcessTreeError> {
        Ok(())
    }
}
impl SandboxedProviderLaunch for Sandbox {
    type Process = Process;
    fn launch_sandboxed(
        &self,
        _: &SandboxedLaunchRequest,
        _: &ProviderLaunch,
    ) -> Result<Process, SandboxedProviderLaunchError> {
        Ok(Process)
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

fn config() -> PrivateClaudeAuthorityConfig<Sandbox> {
    PrivateClaudeAuthorityConfig {
        evidence_record: PathBuf::from("/does-not-exist/claude-evidence.json"),
        trusted_keys: vec![
            "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ],
        coordinator_id: "private-claude-host".into(),
        host_epoch: HostEpoch(1),
        now_unix_seconds: 1,
        sandbox_request: SandboxedLaunchRequest {
            lock: RunVersionLock {
                provider: "claude".into(),
                canonical_path: "/private/gent/claude".into(),
                file_identity: "1:2".into(),
                digest_sha256: "a".repeat(64),
                version: "1.0.0".into(),
                compatibility_entry: "claude-1.0.0".into(),
            },
            profile: sandbox_profile(),
        },
        sandbox_launch: Sandbox,
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
    let mut wrong_provider = config();
    wrong_provider.sandbox_request.lock.provider = "codex".into();
    assert!(matches!(
        validate(&wrong_provider),
        Err(PrivateClaudeAuthorityError::InvalidSandboxRequest)
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
        compose_private_claude_authority(
            &state,
            PrivateClaudeAuthorityConfig {
                sandbox_launch: Sandbox,
                ..config()
            },
        ),
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
