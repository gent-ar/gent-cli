//! Standalone Claude lifecycle composition from one explicitly selected local executable.
//!
//! This is the public Gent CLI path: it has no release artifact, package discovery, or PATH
//! rechecked by the resolver and again by the process runner immediately before launch.

use std::{path::PathBuf, sync::Arc};

use gent_drivers::{PublicProvider, buffering::BufferPolicy, supervisor::ProcessLauncher};
use gent_ports::PublicProviderResolver;
use gent_runtime::{Coordinator, GoalAuthority, GoalService};
use gent_store::SqliteLedger;
use gent_types::HostEpoch;

use crate::{
    approved_claude_host::ApprovedClaudeHost,
    authority_profile::{
        AuthorityProfileConfig, AuthorityProfileError, PublicDriverApproval, PublicDriverRequest,
        ValidatedAuthorityProfile,
    },
    claude_authority_supervisor::{
        PrivateClaudeEscalation, PrivateClaudeShutdown, PrivateClaudeSupervisor, PrivateClaudeWake,
    },
    claude_prompt_lifecycle::ClaudePromptRunner,
    claude_summary_runner::{ClaudeSummaryRunner, ClaudeSummarySchedulerHook},
    local_provider_locks::{LocalProviderLockError, LocalProviderLocks},
    private_lifecycle_loop::PrivateLifecycleOwner,
    public_driver_runtime::{PublicDriversRuntime, PublicDriversRuntimeError},
};

const BUFFERED_FRAMES: usize = 16;
const BUFFERED_BYTES: usize = 256 * 1024;
const MAX_ACTIVE_CLAUDE_RUNS: usize = 4;
const STANDALONE_EVIDENCE_REFERENCE: &str = "standalone-local-claude-v1";

/// Explicit local inputs for the standalone Claude provider host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StandaloneClaudeConfig {
    pub(crate) data_dir: PathBuf,
    pub(crate) coordinator_id: String,
    pub(crate) host_epoch: HostEpoch,
    pub(crate) executable: PathBuf,
    pub(crate) mcp_config: Option<PathBuf>,
}

type StandaloneClaudeRunner<L> = ClaudePromptRunner<L, <L as ProcessLauncher>::Process>;

/// Claude lifecycle owner backed only by an explicitly selected local executable.
pub(crate) struct StandaloneClaudeHost<L: ProcessLauncher> {
    supervisor:
        PrivateClaudeSupervisor<SqliteLedger, StandaloneClaudeRunner<L>, LocalProviderLocks>,
}

impl<L> StandaloneClaudeHost<L>
where
    L: ProcessLauncher + 'static,
{
    pub(crate) fn wake(&mut self) -> Result<PrivateClaudeWake, gent_runtime::RuntimeError> {
        self.supervisor.wake()
    }

    pub(crate) fn request_shutdown(
        &mut self,
    ) -> Result<PrivateClaudeShutdown, gent_runtime::RuntimeError> {
        self.supervisor.request_shutdown()
    }

    pub(crate) fn respond_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), gent_runtime::RuntimeError> {
        self.supervisor
            .respond_permission(run_id, request_id, behavior, persist_suggestions)
    }

    pub(crate) fn escalate_shutdown(
        &mut self,
    ) -> Result<PrivateClaudeEscalation, gent_runtime::RuntimeError> {
        self.supervisor.escalate_shutdown()
    }
}

impl<L> PrivateLifecycleOwner for StandaloneClaudeHost<L>
where
    L: ProcessLauncher + 'static,
{
    type Wake = PrivateClaudeWake;
    type Shutdown = PrivateClaudeShutdown;
    type Escalation = PrivateClaudeEscalation;
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

    fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), ()> {
        self.respond_claude_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            None,
        )
    }

    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), ()> {
        self.supervisor
            .respond_permission_with_input(
                run_id,
                request_id,
                behavior,
                persist_suggestions,
                updated_input,
            )
            .map_err(|_| ())
    }

    fn interrupt_run(&mut self, run_id: &str) -> Result<(), ()> {
        self.supervisor.interrupt_run(run_id).map_err(|_| ())
    }
}

/// Failure before a standalone Claude lifecycle becomes reachable.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StandaloneClaudeError {
    #[error("standalone Claude coordinator identity must be bounded and nonempty")]
    InvalidCoordinator,
    #[error(transparent)]
    LocalLock(#[from] LocalProviderLockError),
    #[error(transparent)]
    Profile(#[from] AuthorityProfileError),
    #[error(transparent)]
    Runtime(#[from] PublicDriversRuntimeError),
    #[error("Claude summary runner is unavailable: {0}")]
    Summary(String),
}

/// Composes the real Claude lifecycle from a selected local executable.
///
/// No process is started here.  Recovery and launch remain under the returned owner, so callers
/// can retain the same durable conversation and provider lifecycle semantics as app integration.
pub(crate) fn compose_standalone_claude<L>(
    ledger: SqliteLedger,
    coordinator: Coordinator<SqliteLedger>,
    config: &StandaloneClaudeConfig,
    launcher: L,
) -> Result<StandaloneClaudeHost<L>, StandaloneClaudeError>
where
    L: ProcessLauncher + 'static,
{
    validate(config)?;
    let resolver =
        LocalProviderLocks::capture([(PublicProvider::Claude, config.executable.clone())])?;
    let summary_lock = resolver
        .resolve("claude")
        .map_err(|error| StandaloneClaudeError::Summary(error.to_string()))?;
    let summary_hook = Arc::new(ClaudeSummarySchedulerHook::new(
        ledger.clone(),
        ClaudeSummaryRunner::new(summary_lock)
            .map_err(|error| StandaloneClaudeError::Summary(error.to_string()))?,
    ));
    let runner = ClaudePromptRunner::new(
        launcher,
        BufferPolicy::new(BUFFERED_FRAMES, BUFFERED_BYTES, 0, 0)
            .expect("fixed standalone Claude buffer policy is valid"),
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
    Ok(StandaloneClaudeHost {
        supervisor: PrivateClaudeSupervisor::new(ApprovedClaudeHost::new(
            runtime,
            config.coordinator_id.clone(),
            config.host_epoch,
            MAX_ACTIVE_CLAUDE_RUNS,
            Some(summary_hook),
        )),
    })
}

fn validate(config: &StandaloneClaudeConfig) -> Result<(), StandaloneClaudeError> {
    (!config.coordinator_id.trim().is_empty() && config.coordinator_id.len() <= 256)
        .then_some(())
        .ok_or(StandaloneClaudeError::InvalidCoordinator)
}

fn profile() -> Result<ValidatedAuthorityProfile, StandaloneClaudeError> {
    AuthorityProfileConfig {
        public_drivers: PublicDriverRequest::Approved(PublicDriverApproval {
            evidence_reference: STANDALONE_EVIDENCE_REFERENCE.into(),
            compatibility_manifest_sha256: "0".repeat(64),
        }),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .map_err(StandaloneClaudeError::from)
}

#[cfg(test)]
#[path = "claude_standalone_authority_tests.rs"]
mod tests;
