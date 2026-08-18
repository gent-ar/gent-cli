//! Private, fail-closed Claude authority composition.
//!
//! This dormant seam is never selected by daemon bootstrap or a command-line argument. A future
//! private owner must provide signed Claude evidence, a locked private prefix, and a native
//! sandbox preflight implementation before it can construct a Claude scheduler host.

use std::path::PathBuf;
use std::sync::Arc;

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::{SystemLauncher, SystemProcess};
use gent_ports::SandboxedProviderPreflight;
use gent_runtime::{GoalAuthority, GoalService};
use gent_store::SqliteLedger;
use gent_types::{HostEpoch, SandboxLaunchProfile};

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
    pub(crate) sandbox_profile: SandboxLaunchProfile,
    pub(crate) sandbox_preflight: S,
}

/// The only system-backed Claude authority host this seam can construct.
pub(crate) type PrivateClaudeAuthorityHost<S> = ApprovedClaudeHost<
    SqliteLedger,
    SandboxedClaudePromptExecution<ClaudePromptRunner<SystemLauncher, SystemProcess>, S>,
    ClaudeOnlyResolver<PrivatePrefixDiscovery, SystemVersionProbe>,
>;

/// Failure before a private Claude authority host becomes reachable.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PrivateClaudeAuthorityError {
    #[error("private Claude coordinator identity must be bounded and nonempty")]
    InvalidCoordinator,
    #[error(transparent)]
    Preflight(#[from] ClaudeAuthorityPreflightError),
    #[error(transparent)]
    Profile(#[from] AuthorityProfileError),
    #[error(transparent)]
    Runtime(#[from] PublicDriversRuntimeError),
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
        config.sandbox_profile,
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
    Ok(ApprovedClaudeHost::new(
        runtime,
        config.coordinator_id,
        config.host_epoch,
        MAX_ACTIVE_CLAUDE_RUNS,
    ))
}

fn validate<S>(
    config: &PrivateClaudeAuthorityConfig<S>,
) -> Result<(), PrivateClaudeAuthorityError> {
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
