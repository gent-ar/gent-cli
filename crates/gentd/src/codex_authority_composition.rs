//! Private, fail-closed Codex authority composition.
//!
//! This is deliberately not selected by daemon arguments. A future private supervisor must hand
//! it a signed evidence record and compatibility envelope before it can construct a process
//! runner. The ordinary `--agent-chat-authority` profile remains durable-chat-only.

use std::path::PathBuf;
use std::sync::Arc;

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::codex_prompt_runner::CodexPromptRunner;
use gent_drivers::supervisor::ProcessLauncher;
use gent_runtime::{GoalAuthority, GoalService};
use gent_store::SqliteLedger;
use gent_types::HostEpoch;

use crate::approved_codex_host::ApprovedCodexHost;
use crate::authority_profile::{
    AuthorityProfileConfig, AuthorityProfileError, PublicDriverApproval, PublicDriverRequest,
    ValidatedAuthorityProfile,
};
use crate::codex_authority_preflight::{self, CodexAuthorityPreflightError};
use crate::codex_authority_supervisor::{
    PrivateCodexEscalation, PrivateCodexShutdown, PrivateCodexSupervisor, PrivateCodexWake,
};
use crate::locked_provider_resolver::LockedProviderResolver;
use crate::private_lifecycle_loop::PrivateLifecycleOwner;
use crate::provider_resolver::CodexOnlyResolver;
use crate::public_driver_runtime::{PublicDriversRuntime, PublicDriversRuntimeError};
use crate::runtime_facade::DaemonCompositionState;

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;
const BUFFERED_FRAMES: usize = 16;
const BUFFERED_BYTES: usize = 256 * 1024;
const MAX_ACTIVE_CODEX_RUNS: usize = 4;
const EVIDENCE_REFERENCE: &str = "private-codex-authority-v1";

type PrivateCodexRunner<L> = CodexPromptRunner<L, <L as ProcessLauncher>::Process>;

/// Private supervisor inputs for the one Codex-only process authority profile.
///
/// This is intentionally not a public command-line configuration. The executable comes only
/// from the daemon's durable provisioned-installation ledger, never `PATH` or an app provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateCodexAuthorityConfig {
    pub(crate) evidence_record: PathBuf,
    pub(crate) trusted_keys: Vec<String>,
    pub(crate) coordinator_id: String,
    pub(crate) host_epoch: HostEpoch,
    pub(crate) now_unix_seconds: u64,
}

/// Private authority host whose launcher is selected only by daemon composition.
pub(crate) struct PrivateCodexAuthorityHost<L: ProcessLauncher> {
    supervisor: PrivateCodexSupervisor<
        SqliteLedger,
        PrivateCodexRunner<L>,
        CodexOnlyResolver<LockedProviderResolver<SqliteLedger>>,
    >,
}

impl<L> PrivateCodexAuthorityHost<L>
where
    L: ProcessLauncher + 'static,
{
    /// Drives one recovery/tick/drain pass through the only private lifecycle owner.
    pub(crate) fn wake(&mut self) -> Result<PrivateCodexWake, gent_runtime::RuntimeError> {
        self.supervisor.wake()
    }

    /// Starts a durable process-tree drain without accepting another provider prompt.
    pub(crate) fn request_shutdown(
        &mut self,
    ) -> Result<PrivateCodexShutdown, gent_runtime::RuntimeError> {
        self.supervisor.request_shutdown()
    }

    /// Advances a caller-timed process-tree escalation ladder.
    pub(crate) fn escalate_shutdown(
        &mut self,
    ) -> Result<PrivateCodexEscalation, gent_runtime::RuntimeError> {
        self.supervisor.escalate_shutdown()
    }
}

impl<L> PrivateLifecycleOwner for PrivateCodexAuthorityHost<L>
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
}

/// Failure before a private Codex authority host becomes reachable.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrivateCodexAuthorityError {
    #[error("private Codex coordinator identity must be bounded and nonempty")]
    InvalidCoordinator,
    #[error(transparent)]
    Preflight(#[from] CodexAuthorityPreflightError),
    #[error(transparent)]
    Profile(#[from] AuthorityProfileError),
    #[error(transparent)]
    Runtime(#[from] PublicDriversRuntimeError),
}

/// Composes a Codex-only lifecycle after loading fresh signed evidence.
///
/// Evidence and the exact compatibility envelope are revalidated before this function creates a
/// resolver or runner. The sealed ordinary composition supplies its read-only launcher; any
/// future broad composition must supply an independently enforced containment launcher.
/// This does not discover, probe, launch, or advertise a provider; the caller must retain the
/// returned host and schedule its bounded `wake` method. Its lifecycle cannot be driven around
/// the retained supervisor, but it still has no prompt wake source or daemon bootstrap wiring.
///
/// # Errors
/// Returns before runner construction if the coordinator or signed evidence/compatibility fence
/// is invalid, and otherwise returns an authority-composition failure.
pub(crate) fn compose_private_codex_authority<L>(
    state: &DaemonCompositionState,
    config: &PrivateCodexAuthorityConfig,
    launcher: L,
) -> Result<PrivateCodexAuthorityHost<L>, PrivateCodexAuthorityError>
where
    L: ProcessLauncher + 'static,
{
    validate(config)?;
    let preflight = codex_authority_preflight::load(
        &config.evidence_record,
        &config.trusted_keys,
        state.compatibility(),
        config.now_unix_seconds,
    )?;
    let profile = profile(preflight.evidence().compatibility_manifest_sha256())?;
    let runner = CodexPromptRunner::new(
        launcher,
        BufferPolicy::new(BUFFERED_FRAMES, BUFFERED_BYTES, 0, 0)
            .expect("fixed Codex authority buffer policy is valid"),
    );
    let resolver = CodexOnlyResolver::new(LockedProviderResolver::new(state.ledger().clone()));
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
    Ok(PrivateCodexAuthorityHost {
        supervisor: PrivateCodexSupervisor::new(ApprovedCodexHost::new(
            runtime,
            config.coordinator_id.clone(),
            config.host_epoch,
            MAX_ACTIVE_CODEX_RUNS,
        )),
    })
}

fn validate(config: &PrivateCodexAuthorityConfig) -> Result<(), PrivateCodexAuthorityError> {
    (!config.coordinator_id.trim().is_empty() && config.coordinator_id.len() <= 256)
        .then_some(())
        .ok_or(PrivateCodexAuthorityError::InvalidCoordinator)
}

fn profile(digest: &str) -> Result<ValidatedAuthorityProfile, PrivateCodexAuthorityError> {
    AuthorityProfileConfig {
        public_drivers: PublicDriverRequest::Approved(PublicDriverApproval {
            evidence_reference: EVIDENCE_REFERENCE.into(),
            compatibility_manifest_sha256: digest.into(),
        }),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .map_err(PrivateCodexAuthorityError::from)
}

#[cfg(test)]
#[path = "codex_authority_composition_tests.rs"]
mod tests;
