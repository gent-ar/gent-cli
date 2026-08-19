//! Sealed composition of the dormant ordinary Claude and Codex lifecycle hosts.
//!
//! This module deliberately has no bootstrap, argument, environment, or protocol surface. It
//! first validates both provider evidence inputs, then creates the already bounded private hosts
//! and shares their router with a future facade composition. The returned cadence owner is the
//! only value that can drive that router.

use std::sync::{Arc, Mutex};

use gent_drivers::ReadOnlyHostLauncher;
use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::AgentChatProvider;

use crate::claude_authority_composition::{
    PrivateClaudeAuthorityConfig, PrivateClaudeAuthorityError, compose_private_claude_authority,
};
use crate::claude_authority_preflight::{self, ClaudeAuthorityPreflightError};
use crate::codex_authority_composition::{
    PrivateCodexAuthorityConfig, PrivateCodexAuthorityError, compose_private_codex_authority,
};
use crate::codex_authority_preflight::{self, CodexAuthorityPreflightError};
use crate::ordinary_lifecycle_cadence::{
    OrdinaryLifecycleCadence, OrdinaryPromptWake, pair as cadence_pair,
};
use crate::ordinary_lifecycle_router::{OrdinaryProviderHost, OrdinaryPublicLifecycleRouter};
use crate::provider_lifecycle_host::ProviderLifecycleHost;
use crate::runtime_facade::DaemonCompositionState;

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;

/// Private inputs for the simultaneous ordinary Claude and Codex composition.
///
/// Neither configuration is serializable or accepted by the shipped daemon. Their coordinator
/// and epoch must match, so one router cannot accidentally operate two durable owners.
#[derive(Debug)]
pub(crate) struct OrdinaryAuthorityConfig {
    pub(crate) codex: PrivateCodexAuthorityConfig,
    pub(crate) claude: PrivateClaudeAuthorityConfig,
}

/// Failure before or while assembling the dormant ordinary lifecycle router.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OrdinaryAuthorityError {
    #[error("ordinary Claude and Codex coordinators must match")]
    CoordinatorMismatch,
    #[error("ordinary Claude and Codex host epochs must match")]
    HostEpochMismatch,
    #[error(transparent)]
    CodexPreflight(#[from] CodexAuthorityPreflightError),
    #[error(transparent)]
    ClaudePreflight(#[from] ClaudeAuthorityPreflightError),
    #[error(transparent)]
    Codex(#[from] PrivateCodexAuthorityError),
    #[error(transparent)]
    Claude(#[from] PrivateClaudeAuthorityError),
    #[error("ordinary lifecycle router is unavailable")]
    RouterUnavailable,
}

/// Caller-timed bounded cadence for one shared ordinary-provider router.
///
/// This contains no daemon task or timer. A later approved bootstrap must retain this value and
/// choose its cadence; the hard-observer bootstrap never constructs it.
#[derive(Clone, Debug)]
pub(crate) struct OrdinaryAuthorityRuntime {
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>>,
    prompt_wake: OrdinaryPromptWake<SqliteLedger>,
    cadence: OrdinaryLifecycleCadence<SqliteLedger>,
}

impl OrdinaryAuthorityRuntime {
    /// Returns the exact shared router intended for a future `RuntimeFacade` composition.
    #[must_use]
    pub(crate) fn router(&self) -> Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>> {
        Arc::clone(&self.router)
    }

    /// Returns the paired post-commit wake adapter for the authority-bound facade.
    #[must_use]
    pub(crate) fn prompt_wake(&self) -> OrdinaryPromptWake<SqliteLedger> {
        self.prompt_wake.clone()
    }

    /// Runs recovery and the demand-driven lifecycle cadence for this authority.
    pub(crate) async fn run_cadence(&self) {
        self.cadence.clone().run().await;
    }

    /// Drives every pre-approved host at most once.
    ///
    /// Hosts remain inactive until the post-commit prompt wake reaches their router, so this
    /// cadence cannot launch a provider before the durable prompt transaction completes.
    pub(crate) fn drive_once(&self) -> Result<bool, OrdinaryAuthorityError> {
        self.router
            .lock()
            .map_err(|_| OrdinaryAuthorityError::RouterUnavailable)?
            .drive_once()
            .map_err(|_| OrdinaryAuthorityError::RouterUnavailable)
    }
}

/// Preflights and composes the private ordinary Claude/Codex lifecycle without bootstrap wiring.
///
/// Both evidence records are read and validated before either private resolver or launcher is
/// constructed. Each provider-specific composition repeats its preflight immediately
/// before construction, closing the read-to-compose interval without accepting a stale result.
///
/// # Errors
/// Returns before any private-prefix discovery or provider launch if either preflight fails.
pub(crate) fn compose_ordinary_authority(
    state: &DaemonCompositionState,
    config: OrdinaryAuthorityConfig,
) -> Result<OrdinaryAuthorityRuntime, OrdinaryAuthorityError> {
    validate_shared_owner(&config)?;
    preflight_all(state, &config)?;
    let launcher = ReadOnlyHostLauncher::new(STREAM_CAPTURE_BYTES);
    let codex = compose_private_codex_authority(state, &config.codex, launcher)?;
    let claude = compose_private_claude_authority(state, config.claude, launcher)?;
    let router = OrdinaryPublicLifecycleRouter::new(
        AgentChatReadService::new(state.ledger().clone()),
        vec![
            Box::new(OrdinaryProviderHost::new(
                AgentChatProvider::Codex,
                ProviderLifecycleHost::new(codex),
            )),
            Box::new(OrdinaryProviderHost::new(
                AgentChatProvider::Claude,
                ProviderLifecycleHost::new(claude),
            )),
        ],
    )
    .map_err(|_| OrdinaryAuthorityError::RouterUnavailable)?;
    let router = Arc::new(Mutex::new(router));
    let (prompt_wake, cadence) = cadence_pair(Arc::clone(&router));
    Ok(OrdinaryAuthorityRuntime {
        router,
        prompt_wake,
        cadence,
    })
}

fn validate_shared_owner(config: &OrdinaryAuthorityConfig) -> Result<(), OrdinaryAuthorityError> {
    (config.codex.coordinator_id == config.claude.coordinator_id)
        .then_some(())
        .ok_or(OrdinaryAuthorityError::CoordinatorMismatch)?;
    (config.codex.host_epoch == config.claude.host_epoch)
        .then_some(())
        .ok_or(OrdinaryAuthorityError::HostEpochMismatch)
}

fn preflight_all(
    state: &DaemonCompositionState,
    config: &OrdinaryAuthorityConfig,
) -> Result<(), OrdinaryAuthorityError> {
    codex_authority_preflight::load(
        &config.codex.evidence_record,
        &config.codex.trusted_keys,
        state.compatibility(),
        config.codex.now_unix_seconds,
    )?;
    claude_authority_preflight::load(
        &config.claude.evidence_record,
        &config.claude.trusted_keys,
        state.compatibility(),
        config.claude.now_unix_seconds,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "ordinary_authority_composition_tests.rs"]
mod tests;
