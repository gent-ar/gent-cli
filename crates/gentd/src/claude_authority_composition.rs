//! Private, fail-closed Claude authority composition.
//!
//! This dormant seam is never selected by daemon bootstrap or a command-line argument. A future
//! private owner must provide signed Claude evidence and a durable locked installation before it
//! can construct a Claude scheduler host.

use std::sync::Arc;

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::supervisor::ProcessLauncher;
use gent_runtime::{GoalAuthority, GoalService};
use gent_store::SqliteLedger;
use gent_types::HostEpoch;

use crate::approved_claude_host::ApprovedClaudeHost;
use crate::authority_profile::{
    AuthorityProfileConfig, AuthorityProfileError, PublicDriverApproval, PublicDriverRequest,
    ValidatedAuthorityProfile,
};
use crate::claude_authority_preflight::ClaudeAuthorityPreflight;
use crate::claude_authority_supervisor::{
    PrivateClaudeEscalation, PrivateClaudeShutdown, PrivateClaudeSupervisor, PrivateClaudeWake,
};
use crate::claude_private_resolver::ClaudeOnlyResolver;
use crate::claude_prompt_lifecycle::ClaudePromptRunner;
use crate::locked_provider_resolver::LockedProviderResolver;
use crate::private_lifecycle_loop::PrivateLifecycleOwner;
use crate::public_driver_runtime::{PublicDriversRuntime, PublicDriversRuntimeError};
use crate::runtime_facade::DaemonCompositionState;

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;
const BUFFERED_FRAMES: usize = 16;
const BUFFERED_BYTES: usize = 256 * 1024;
const MAX_ACTIVE_CLAUDE_RUNS: usize = 4;
const EVIDENCE_REFERENCE: &str = "private-claude-authority-v1";

/// Private supervisor inputs for a single Claude-only process authority profile.
///
/// This value is not protocol-serializable and no shipped daemon command constructs it. The
/// launcher is selected only by Gent daemon composition.
#[derive(Debug)]
pub(crate) struct PrivateClaudeAuthorityConfig {
    pub(crate) coordinator_id: String,
    pub(crate) host_epoch: HostEpoch,
}

type PrivateClaudeRunner<L> = ClaudePromptRunner<L, <L as ProcessLauncher>::Process>;

/// Private authority host whose launcher is selected only by daemon composition.
pub(crate) struct PrivateClaudeAuthorityHost<L: ProcessLauncher> {
    supervisor: PrivateClaudeSupervisor<
        SqliteLedger,
        PrivateClaudeRunner<L>,
        ClaudeOnlyResolver<LockedProviderResolver<SqliteLedger>>,
    >,
}

impl<L> PrivateClaudeAuthorityHost<L>
where
    L: ProcessLauncher + 'static,
{
    /// Drives one recovery/tick/drain pass through the only private lifecycle owner.
    pub(crate) fn wake(&mut self) -> Result<PrivateClaudeWake, gent_runtime::RuntimeError> {
        self.supervisor.wake()
    }

    /// Starts a durable process-tree drain without accepting another provider prompt.
    pub(crate) fn request_shutdown(
        &mut self,
    ) -> Result<PrivateClaudeShutdown, gent_runtime::RuntimeError> {
        self.supervisor.request_shutdown()
    }

    /// Advances a caller-timed process-tree escalation ladder.
    pub(crate) fn escalate_shutdown(
        &mut self,
    ) -> Result<PrivateClaudeEscalation, gent_runtime::RuntimeError> {
        self.supervisor.escalate_shutdown()
    }
}

impl<L> PrivateLifecycleOwner for PrivateClaudeAuthorityHost<L>
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
}

/// Failure before a private Claude authority host becomes reachable.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrivateClaudeAuthorityError {
    #[error("private Claude coordinator identity must be bounded and nonempty")]
    InvalidCoordinator,
    #[error(transparent)]
    Profile(#[from] AuthorityProfileError),
    #[error(transparent)]
    Runtime(#[from] PublicDriversRuntimeError),
}

/// Composes a mode-gated Claude lifecycle from a release-verified evidence grant.
///
/// The authority-release reader verifies evidence and its exact compatibility envelope before it
/// supplies this opaque grant. The caller selects the mode-compatible launcher.
/// It does not discover, probe, launch, or advertise Claude. A caller must retain the returned
/// host and schedule its bounded recovery and ticks;
/// daemon bootstrap must not compose it until every authority/evidence gate is proven.
///
/// # Errors
/// Returns before process-runner construction if coordinator identity or evidence validation
/// fails, and otherwise returns only an authority-composition failure.
pub(crate) fn compose_private_claude_authority<L>(
    state: &DaemonCompositionState,
    config: &PrivateClaudeAuthorityConfig,
    preflight: &ClaudeAuthorityPreflight,
    launcher: L,
) -> Result<PrivateClaudeAuthorityHost<L>, PrivateClaudeAuthorityError>
where
    L: ProcessLauncher + 'static,
{
    validate(config)?;
    let profile = profile(preflight.evidence().compatibility_manifest_sha256())?;
    let runner = ClaudePromptRunner::new(
        launcher,
        BufferPolicy::new(BUFFERED_FRAMES, BUFFERED_BYTES, 0, 0)
            .expect("fixed Claude authority buffer policy is valid"),
    );
    let resolver = ClaudeOnlyResolver::new(LockedProviderResolver::new(state.ledger().clone()));
    let goals = Arc::new(GoalService::new(
        state.ledger().clone(),
        GoalAuthority::Approved,
    ));
    let runtime = PublicDriversRuntime::new_with_current_compatibility(
        profile,
        state.coordinator().clone(),
        state.ledger().clone(),
        state.compatibility().clone(),
        runner,
        resolver,
    )?
    .with_active_goal_resolver(goals);
    Ok(PrivateClaudeAuthorityHost {
        supervisor: PrivateClaudeSupervisor::new(ApprovedClaudeHost::new(
            runtime,
            config.coordinator_id.clone(),
            config.host_epoch,
            MAX_ACTIVE_CLAUDE_RUNS,
        )),
    })
}

fn validate(config: &PrivateClaudeAuthorityConfig) -> Result<(), PrivateClaudeAuthorityError> {
    (!config.coordinator_id.trim().is_empty() && config.coordinator_id.len() <= 256)
        .then_some(())
        .ok_or(PrivateClaudeAuthorityError::InvalidCoordinator)
}

fn profile(digest: &str) -> Result<ValidatedAuthorityProfile, PrivateClaudeAuthorityError> {
    AuthorityProfileConfig {
        public_drivers: PublicDriverRequest::Approved(PublicDriverApproval {
            evidence_reference: EVIDENCE_REFERENCE.into(),
            compatibility_manifest_sha256: digest.into(),
        }),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .map_err(PrivateClaudeAuthorityError::from)
}

#[cfg(test)]
#[path = "claude_authority_composition_tests.rs"]
mod tests;
