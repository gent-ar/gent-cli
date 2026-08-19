use std::path::PathBuf;

use gent_drivers::interrupt::{ProcessTreeControl, ProcessTreeError, ProcessTreeSignal};
use gent_drivers::supervisor::{ProviderLaunch, ProviderProcess};
use gent_drivers::{SandboxedProviderLaunch, SandboxedProviderLaunchError};
use gent_runtime::catalog::declared_capabilities;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, HostEpoch,
    SandboxLaunchPolicy, SandboxNetworkPolicy, SandboxResourceLimits, SandboxedLaunchRequest,
};

use super::{OrdinaryAuthorityConfig, OrdinaryAuthorityError, compose_ordinary_authority};
use crate::CompatibilityAssessment;
use crate::claude_authority_composition::PrivateClaudeAuthorityConfig;
use crate::codex_authority_composition::PrivateCodexAuthorityConfig;
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

fn policy() -> SandboxLaunchPolicy {
    SandboxLaunchPolicy::new(
        vec![],
        SandboxNetworkPolicy::Disabled,
        SandboxResourceLimits {
            max_processes: 4,
            max_memory_bytes: 1,
            max_cpu_time_ms: 1,
        },
    )
    .unwrap()
}

fn config() -> OrdinaryAuthorityConfig<Sandbox> {
    OrdinaryAuthorityConfig {
        codex: PrivateCodexAuthorityConfig {
            evidence_record: PathBuf::from("/does-not-exist/codex-evidence.json"),
            trusted_keys: vec![
                "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            ],
            coordinator_id: "ordinary-owner".into(),
            host_epoch: HostEpoch(1),
            now_unix_seconds: 1,
            sandbox_policy: policy(),
        },
        claude: PrivateClaudeAuthorityConfig {
            evidence_record: PathBuf::from("/does-not-exist/claude-evidence.json"),
            trusted_keys: vec![
                "key:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            ],
            coordinator_id: "ordinary-owner".into(),
            host_epoch: HostEpoch(1),
            now_unix_seconds: 1,
            sandbox_policy: policy(),
            sandbox_launch: Sandbox,
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
        &declared_capabilities(),
        CompatibilityAssessment::default(),
    )
    .unwrap();

    assert!(matches!(
        compose_ordinary_authority(&state, config(), Sandbox),
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
        &declared_capabilities(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let mut input = config();
    input.claude.coordinator_id = "different-owner".into();

    assert!(matches!(
        compose_ordinary_authority(&state, input, Sandbox),
        Err(OrdinaryAuthorityError::CoordinatorMismatch)
    ));
    assert!(!state.data_dir().join("providers").exists());
}

#[test]
fn unsupported_selection_fails_before_reading_evidence_or_constructing_a_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let state = DaemonCompositionState::open(
        directory.path(),
        &declared_capabilities(),
        CompatibilityAssessment::default(),
    )
    .unwrap();
    let mut input = config();
    input.selections[0].provider = AgentChatProvider::Claurst;

    assert!(matches!(
        compose_ordinary_authority(&state, input, Sandbox),
        Err(OrdinaryAuthorityError::UnsupportedSelection)
    ));
    assert!(!state.data_dir().join("providers").exists());
}
