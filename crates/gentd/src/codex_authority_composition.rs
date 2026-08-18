//! Private, fail-closed Codex authority composition.
//!
//! This is deliberately not selected by daemon arguments. A future private supervisor must hand
//! it a signed evidence record and compatibility envelope before it can construct a process
//! runner. The ordinary `--agent-chat-authority` profile remains durable-chat-only.

use std::path::PathBuf;

use gent_drivers::buffering::BufferPolicy;
use gent_drivers::codex_prompt_runner::CodexPromptRunner;
use gent_drivers::{SystemLauncher, SystemProcess};
use gent_store::SqliteLedger;
use gent_types::HostEpoch;

use crate::approved_codex_host::ApprovedCodexHost;
use crate::authority_profile::{
    AuthorityProfileConfig, AuthorityProfileError, PublicDriverApproval, PublicDriverRequest,
    ValidatedAuthorityProfile,
};
use crate::codex_authority_preflight::{self, CodexAuthorityPreflightError};
use crate::provider_resolver::{
    CodexOnlyResolver, DaemonProviderResolver, PrivatePrefixDiscovery, SystemVersionProbe,
};
use crate::public_driver_runtime::{PublicDriversRuntime, PublicDriversRuntimeError};
use crate::runtime_facade::DaemonCompositionState;

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;
const BUFFERED_FRAMES: usize = 16;
const BUFFERED_BYTES: usize = 256 * 1024;
const MAX_ACTIVE_CODEX_RUNS: usize = 4;
const EVIDENCE_REFERENCE: &str = "private-codex-authority-v1";

/// Private supervisor inputs for the one Codex-only process authority profile.
///
/// This is intentionally not a public command-line configuration. The prefix is derived from
/// the single daemon composition state and cannot be redirected to `PATH` or an app provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateCodexAuthorityConfig {
    pub(crate) evidence_record: PathBuf,
    pub(crate) trusted_keys: Vec<String>,
    pub(crate) coordinator_id: String,
    pub(crate) working_directory: Option<String>,
    pub(crate) host_epoch: HostEpoch,
    pub(crate) now_unix_seconds: u64,
}

/// The only system-backed authority host this seam can construct.
pub(crate) type PrivateCodexAuthorityHost = ApprovedCodexHost<
    SqliteLedger,
    CodexPromptRunner<SystemLauncher, SystemProcess>,
    CodexOnlyResolver<PrivatePrefixDiscovery, SystemVersionProbe>,
>;

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
/// resolver or runner. It does not discover, probe, launch, or advertise a provider; the caller
/// must retain the returned host and schedule its bounded `recover` and `tick` methods. This is
/// not yet a supervisor: it has no prompt wake source, shutdown request, or timer-driven process
/// tree escalation/drain loop, so daemon bootstrap must not compose it.
///
/// # Errors
/// Returns before runner construction if the coordinator or signed evidence/compatibility fence
/// is invalid, and otherwise returns an authority-composition failure.
pub(crate) fn compose_private_codex_authority(
    state: &DaemonCompositionState,
    config: &PrivateCodexAuthorityConfig,
) -> Result<PrivateCodexAuthorityHost, PrivateCodexAuthorityError> {
    validate(config)?;
    let preflight = codex_authority_preflight::load(
        &config.evidence_record,
        &config.trusted_keys,
        state.compatibility(),
        config.now_unix_seconds,
    )?;
    let profile = profile(preflight.evidence().compatibility_manifest_sha256())?;
    let runner = CodexPromptRunner::new(
        SystemLauncher::new(STREAM_CAPTURE_BYTES),
        BufferPolicy::new(BUFFERED_FRAMES, BUFFERED_BYTES, 0, 0)
            .expect("fixed Codex authority buffer policy is valid"),
    );
    let prefix = state.data_dir().join("providers").join("npm-global");
    let resolver = CodexOnlyResolver::new(DaemonProviderResolver::new(
        state.compatibility().clone(),
        PrivatePrefixDiscovery::new(prefix),
        SystemVersionProbe,
    ));
    let runtime = PublicDriversRuntime::new(
        profile,
        state.coordinator().clone(),
        state.ledger().clone(),
        state.compatibility().clone(),
        runner,
        resolver,
    )?;
    Ok(ApprovedCodexHost::new(
        runtime,
        config.coordinator_id.clone(),
        config.working_directory.clone(),
        config.host_epoch,
        MAX_ACTIVE_CODEX_RUNS,
    ))
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
