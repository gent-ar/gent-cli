//! Private, fail-closed Claude authority composition.
//!
//! This dormant seam is never selected by daemon bootstrap or a command-line argument. A future
//! private owner must provide signed Claude evidence, a locked private prefix, and a native
//! sandbox preflight implementation before it can construct a Claude scheduler host.

use std::path::PathBuf;
use std::sync::Arc;

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::{SystemLauncher, SystemProcess};
use gent_ports::{SandboxedProviderPreflight, SandboxedProviderPreflightError};
use gent_runtime::{GoalAuthority, GoalService};
use gent_store::SqliteLedger;
use gent_types::{HostEpoch, SandboxLaunchAttestation, SandboxedLaunchRequest};

use crate::approved_claude_host::ApprovedClaudeHost;
use crate::authority_profile::{
    AuthorityProfileConfig, AuthorityProfileError, PublicDriverApproval, PublicDriverRequest,
    ValidatedAuthorityProfile,
};
use crate::claude_authority_preflight::{self, ClaudeAuthorityPreflightError};
use crate::claude_private_resolver::ClaudeOnlyResolver;
use crate::claude_prompt_lifecycle::{ClaudePromptRunner, SandboxedClaudePromptExecution};
use crate::provider_resolver::{
    DaemonProviderResolver, PrivatePrefixDiscovery, SystemVersionProbe,
};
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
/// sandbox profile and preflight are supplied only by a future trusted composition owner.
#[derive(Debug)]
pub(crate) struct PrivateClaudeAuthorityConfig<S> {
    pub(crate) evidence_record: PathBuf,
    pub(crate) trusted_keys: Vec<String>,
    pub(crate) coordinator_id: String,
    pub(crate) host_epoch: HostEpoch,
    pub(crate) now_unix_seconds: u64,
    /// An immutable executable lock and credential-free sandbox profile supplied only by Gent.
    pub(crate) sandbox_request: SandboxedLaunchRequest,
    pub(crate) sandbox_preflight: S,
}

type PrivateClaudeHostInner<S> = ApprovedClaudeHost<
    SqliteLedger,
    SandboxedClaudePromptExecution<ClaudePromptRunner<SystemLauncher, SystemProcess>, S>,
    ClaudeOnlyResolver<PrivatePrefixDiscovery, SystemVersionProbe>,
>;

/// Private authority host whose lifecycle is inseparable from its sandbox attestation.
pub(crate) struct PrivateClaudeAuthorityHost<S> {
    host: PrivateClaudeHostInner<S>,
    attestation: SandboxLaunchAttestation,
}

impl<S> PrivateClaudeAuthorityHost<S>
where
    S: SandboxedProviderPreflight + std::fmt::Debug + 'static,
{
    /// Returns the containment proof that gated this host's construction.
    #[must_use]
    pub(crate) fn sandbox_attestation(&self) -> &SandboxLaunchAttestation {
        &self.attestation
    }

    /// Reconciles durable pre-launch claims while retaining the required sandbox gate.
    pub(crate) fn recover(&self) -> Result<(), gent_runtime::RuntimeError> {
        self.host.recover()
    }

    /// Runs one bounded lifecycle pass while retaining the required sandbox gate.
    pub(crate) fn tick(
        &mut self,
    ) -> Result<crate::claude_prompt_lifecycle::ClaudeLifecycleTick, gent_runtime::RuntimeError>
    {
        self.host.tick()
    }

    /// Drains owned provider processes without accepting another prompt.
    pub(crate) fn drain(
        &mut self,
    ) -> Result<crate::approved_claude_host::ApprovedClaudeDrain, gent_runtime::RuntimeError> {
        self.host.drain()
    }
}

/// Failure before a private Claude authority host becomes reachable.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrivateClaudeAuthorityError {
    #[error("private Claude coordinator identity must be bounded and nonempty")]
    InvalidCoordinator,
    #[error("private Claude sandbox request must be bound to the Claude provider")]
    InvalidSandboxRequest,
    #[error(transparent)]
    Preflight(#[from] ClaudeAuthorityPreflightError),
    #[error(transparent)]
    Profile(#[from] AuthorityProfileError),
    #[error(transparent)]
    Runtime(#[from] PublicDriversRuntimeError),
    #[error(transparent)]
    Sandbox(#[from] SandboxedProviderPreflightError),
    #[error("sandbox attestation is not bound to the exact Claude executable and profile")]
    InvalidSandboxAttestation,
}

/// Composes a sandbox-gated Claude lifecycle after loading fresh signed evidence.
///
/// Evidence and the exact compatibility envelope are revalidated before this function creates a
/// private-prefix resolver or a process runner. It does not discover, probe, launch, or advertise
/// Claude. A caller must retain the returned host and schedule its bounded recovery and ticks;
/// daemon bootstrap must not compose it until every authority/evidence gate is proven.
///
/// # Errors
/// Returns before process-runner construction if coordinator identity or evidence validation
/// fails, and otherwise returns only an authority-composition failure.
pub(crate) fn compose_private_claude_authority<S>(
    state: &DaemonCompositionState,
    config: PrivateClaudeAuthorityConfig<S>,
) -> Result<PrivateClaudeAuthorityHost<S>, PrivateClaudeAuthorityError>
where
    S: SandboxedProviderPreflight + std::fmt::Debug + 'static,
{
    validate(&config)?;
    let attestation = config
        .sandbox_preflight
        .preflight(&config.sandbox_request)?;
    validate_attestation(&config.sandbox_request, &attestation)?;
    let preflight = claude_authority_preflight::load(
        &config.evidence_record,
        &config.trusted_keys,
        state.compatibility(),
        config.now_unix_seconds,
    )?;
    let profile = profile(preflight.evidence().compatibility_manifest_sha256())?;
    let runner = SandboxedClaudePromptExecution::new(
        ClaudePromptRunner::new(
            SystemLauncher::new(STREAM_CAPTURE_BYTES),
            BufferPolicy::new(BUFFERED_FRAMES, BUFFERED_BYTES, 0, 0)
                .expect("fixed Claude authority buffer policy is valid"),
        ),
        config.sandbox_preflight,
        config.sandbox_request.profile,
    );
    let prefix = state.data_dir().join("providers").join("npm-global");
    let resolver = ClaudeOnlyResolver::new(DaemonProviderResolver::new(
        state.compatibility().clone(),
        PrivatePrefixDiscovery::new(prefix),
        SystemVersionProbe,
    ));
    let goals = Arc::new(GoalService::new(
        state.ledger().clone(),
        GoalAuthority::Approved,
    ));
    let runtime = PublicDriversRuntime::new(
        profile,
        state.coordinator().clone(),
        state.ledger().clone(),
        state.compatibility().clone(),
        runner,
        resolver,
    )?
    .with_active_goal_resolver(goals);
    Ok(PrivateClaudeAuthorityHost {
        host: ApprovedClaudeHost::new(
            runtime,
            config.coordinator_id,
            config.host_epoch,
            MAX_ACTIVE_CLAUDE_RUNS,
        ),
        attestation,
    })
}

fn validate<S>(
    config: &PrivateClaudeAuthorityConfig<S>,
) -> Result<(), PrivateClaudeAuthorityError> {
    (!config.coordinator_id.trim().is_empty() && config.coordinator_id.len() <= 256)
        .then_some(())
        .ok_or(PrivateClaudeAuthorityError::InvalidCoordinator)?;
    (config.sandbox_request.lock.provider == "claude")
        .then_some(())
        .ok_or(PrivateClaudeAuthorityError::InvalidSandboxRequest)
}

fn validate_attestation(
    request: &SandboxedLaunchRequest,
    attestation: &SandboxLaunchAttestation,
) -> Result<(), PrivateClaudeAuthorityError> {
    (request.lock.digest_sha256 == attestation.executable_digest_sha256
        && request.lock.file_identity == attestation.executable_file_identity
        && request.profile.digest_sha256() == attestation.profile_digest_sha256)
        .then_some(())
        .ok_or(PrivateClaudeAuthorityError::InvalidSandboxAttestation)
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
