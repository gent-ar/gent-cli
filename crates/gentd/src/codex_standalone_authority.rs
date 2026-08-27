//! Standalone Codex lifecycle composition from one explicitly selected local executable.
//!
//! The standalone path has no release artifact and no PATH discovery. Its resolver captures the
//! supplied executable and rechecks that exact identity before every provider launch.

use std::{path::PathBuf, sync::Arc};

use gent_drivers::{
    PublicProvider, buffering::BufferPolicy, codex_prompt_runner::CodexPromptRunner,
    supervisor::ProcessLauncher,
};
use gent_ports::PublicProviderResolver;
use gent_runtime::{Coordinator, GoalAuthority, GoalService};
use gent_store::SqliteLedger;
use gent_types::HostEpoch;

use crate::{
    approved_codex_host::ApprovedCodexHost,
    authority_profile::{
        AuthorityProfileConfig, AuthorityProfileError, PublicDriverApproval, PublicDriverRequest,
        ValidatedAuthorityProfile,
    },
    codex_authority_supervisor::{
        PrivateCodexEscalation, PrivateCodexShutdown, PrivateCodexSupervisor, PrivateCodexWake,
    },
    codex_summary_runner::{CodexSummaryRunner, CodexSummarySchedulerHook},
    local_provider_locks::{LocalProviderLockError, LocalProviderLocks},
    private_lifecycle_loop::PrivateLifecycleOwner,
    public_driver_runtime::{PublicDriversRuntime, PublicDriversRuntimeError},
};

const BUFFERED_FRAMES: usize = 16;
const BUFFERED_BYTES: usize = 256 * 1024;
const MAX_ACTIVE_CODEX_RUNS: usize = 4;
const STANDALONE_EVIDENCE_REFERENCE: &str = "standalone-local-codex-v1";

/// Explicit local inputs for the standalone Codex provider host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StandaloneCodexConfig {
    pub(crate) data_dir: PathBuf,
    pub(crate) coordinator_id: String,
    pub(crate) host_epoch: HostEpoch,
    pub(crate) executable: PathBuf,
    pub(crate) mcp_servers: Option<serde_json::Value>,
    pub(crate) mcp_config: Option<PathBuf>,
}

type StandaloneCodexRunner<L> = CodexPromptRunner<L, <L as ProcessLauncher>::Process>;

/// Codex lifecycle owner backed only by an explicitly selected local executable.
pub(crate) struct StandaloneCodexHost<L: ProcessLauncher> {
    supervisor: PrivateCodexSupervisor<SqliteLedger, StandaloneCodexRunner<L>, LocalProviderLocks>,
}

impl<L> StandaloneCodexHost<L>
where
    L: ProcessLauncher + 'static,
{
    pub(crate) fn wake(&mut self) -> Result<PrivateCodexWake, gent_runtime::RuntimeError> {
        self.supervisor.wake()
    }

    pub(crate) fn request_shutdown(
        &mut self,
    ) -> Result<PrivateCodexShutdown, gent_runtime::RuntimeError> {
        self.supervisor.request_shutdown()
    }

    pub(crate) fn respond_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), gent_runtime::RuntimeError> {
        self.supervisor
            .respond_permission(run_id, request_id, decision, answers)
    }

    pub(crate) fn escalate_shutdown(
        &mut self,
    ) -> Result<PrivateCodexEscalation, gent_runtime::RuntimeError> {
        self.supervisor.escalate_shutdown()
    }
}

impl<L> PrivateLifecycleOwner for StandaloneCodexHost<L>
where
    L: ProcessLauncher + 'static,
{
    type Wake = PrivateCodexWake;
    type Shutdown = PrivateCodexShutdown;
    type Escalation = PrivateCodexEscalation;
    type Error = gent_runtime::RuntimeError;

    fn wake(&mut self) -> Result<Self::Wake, Self::Error> {
        self.supervisor.wake()
    }

    fn request_shutdown(&mut self) -> Result<Self::Shutdown, Self::Error> {
        self.supervisor.request_shutdown()
    }

    fn escalate_shutdown(&mut self) -> Result<Self::Escalation, Self::Error> {
        self.supervisor.escalate_shutdown()
    }

    fn needs_drive(&self) -> bool {
        self.supervisor.needs_drive()
    }

    fn shutdown_complete(&self) -> bool {
        self.supervisor.shutdown_complete()
    }

    fn respond_codex_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), ()> {
        self.respond_permission(run_id, request_id, decision, answers)
            .map_err(|_| ())
    }

    fn interrupt_run(&mut self, run_id: &str) -> Result<(), ()> {
        self.supervisor.interrupt_run(run_id).map_err(|_| ())
    }
}

/// Failure before a standalone Codex lifecycle becomes reachable.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StandaloneCodexError {
    #[error("standalone Codex coordinator identity must be bounded and nonempty")]
    InvalidCoordinator,
    #[error(transparent)]
    LocalLock(#[from] LocalProviderLockError),
    #[error(transparent)]
    Profile(#[from] AuthorityProfileError),
    #[error(transparent)]
    Runtime(#[from] PublicDriversRuntimeError),
    #[error("Codex summary runner is unavailable: {0}")]
    Summary(String),
}

/// Composes the real Codex lifecycle from a selected local executable without release material.
///
/// No provider process is started here. The returned owner retains the existing Codex session
/// resume, durable dispatch, stream normalization, and process-drain lifecycle.
pub(crate) fn compose_standalone_codex<L>(
    ledger: SqliteLedger,
    coordinator: Coordinator<SqliteLedger>,
    config: &StandaloneCodexConfig,
    launcher: L,
) -> Result<StandaloneCodexHost<L>, StandaloneCodexError>
where
    L: ProcessLauncher + Clone + std::fmt::Debug + 'static,
{
    validate(config)?;
    let resolver =
        LocalProviderLocks::capture([(PublicProvider::Codex, config.executable.clone())])?;
    let summary_lock = resolver
        .resolve("codex")
        .map_err(|error| StandaloneCodexError::Summary(error.to_string()))?;
    let summary_hook = Arc::new(CodexSummarySchedulerHook::new(
        ledger.clone(),
        CodexSummaryRunner::new(launcher.clone(), summary_lock, std::env::temp_dir()),
    ));
    let runner = CodexPromptRunner::new(
        launcher,
        BufferPolicy::new(BUFFERED_FRAMES, BUFFERED_BYTES, 0, 0)
            .expect("fixed standalone Codex buffer policy is valid"),
        config.mcp_servers.clone(),
        config.mcp_config.clone(),
    );
    let goals = Arc::new(GoalService::new(ledger.clone(), GoalAuthority::Approved));
    let runtime = PublicDriversRuntime::new_standalone_local(
        profile()?,
        coordinator,
        ledger,
        runner,
        resolver,
    )?
    .with_active_goal_resolver(goals)
    .with_attachment_roots(
        config.data_dir.join("attachments"),
        config.data_dir.join("provider-attachments").join("codex"),
    );
    Ok(StandaloneCodexHost {
        supervisor: PrivateCodexSupervisor::new(
            ApprovedCodexHost::new(
                runtime,
                config.coordinator_id.clone(),
                config.host_epoch,
                MAX_ACTIVE_CODEX_RUNS,
            )
            .with_summary_hook(summary_hook),
        ),
    })
}

fn validate(config: &StandaloneCodexConfig) -> Result<(), StandaloneCodexError> {
    (!config.coordinator_id.trim().is_empty() && config.coordinator_id.len() <= 256)
        .then_some(())
        .ok_or(StandaloneCodexError::InvalidCoordinator)
}

fn profile() -> Result<ValidatedAuthorityProfile, StandaloneCodexError> {
    AuthorityProfileConfig {
        public_drivers: PublicDriverRequest::Approved(PublicDriverApproval {
            evidence_reference: STANDALONE_EVIDENCE_REFERENCE.into(),
            compatibility_manifest_sha256: "0".repeat(64),
        }),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .map_err(StandaloneCodexError::from)
}

#[cfg(test)]
#[path = "codex_standalone_authority_tests.rs"]
mod tests;
