//! Sealed composition of the dormant ordinary Claude and Codex lifecycle hosts.
//!
//! This module deliberately has no bootstrap, argument, environment, or protocol surface. It
//! first validates both provider evidence inputs, then creates the already bounded private hosts
//! and shares their router with a future facade composition. The returned cadence owner is the
//! only value that can drive that router.

use std::sync::{Arc, Mutex};

use gent_drivers::SandboxedProviderLaunch;
use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::{AgentChatMode, AgentChatProvider, AgentChatSelection};

use crate::claude_authority_composition::{
    PrivateClaudeAuthorityConfig, PrivateClaudeAuthorityError, compose_private_claude_authority,
};
use crate::claude_authority_preflight::{self, ClaudeAuthorityPreflightError};
use crate::codex_authority_composition::{
    PrivateCodexAuthorityConfig, PrivateCodexAuthorityError, compose_private_codex_authority,
};
use crate::codex_authority_preflight::{self, CodexAuthorityPreflightError};
use crate::ordinary_lifecycle_router::{OrdinaryProviderHost, OrdinaryPublicLifecycleRouter};
use crate::provider_lifecycle_host::ProviderLifecycleHost;
use crate::runtime_facade::DaemonCompositionState;

/// Private inputs for the simultaneous ordinary Claude and Codex composition.
///
/// Neither configuration is serializable or accepted by the shipped daemon. Their coordinator
/// and epoch must match, so one router cannot accidentally operate two durable owners.
#[derive(Debug)]
pub(crate) struct OrdinaryAuthorityConfig<C> {
    pub(crate) codex: PrivateCodexAuthorityConfig,
    pub(crate) claude: PrivateClaudeAuthorityConfig<C>,
    /// Exact selections an authority facade may permit before a prompt is persisted.
    pub(crate) selections: Vec<AgentChatSelection>,
}

/// Failure before or while assembling the dormant ordinary lifecycle router.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OrdinaryAuthorityError {
    #[error("ordinary Claude and Codex coordinators must match")]
    CoordinatorMismatch,
    #[error("ordinary Claude and Codex host epochs must match")]
    HostEpochMismatch,
    #[error("ordinary authority requires at least one exact approved selection")]
    MissingSelections,
    #[error("ordinary authority selections must be unique")]
    DuplicateSelection,
    #[error("ordinary authority allows only Claude/Codex Ask or Plan selections")]
    UnsupportedSelection,
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
    selections: Vec<AgentChatSelection>,
}

impl OrdinaryAuthorityRuntime {
    /// Returns the exact shared router intended for a future `RuntimeFacade` composition.
    #[must_use]
    pub(crate) fn router(&self) -> Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>> {
        Arc::clone(&self.router)
    }

    /// Returns the exact, validated selection allowlist for the authority-bound facade.
    #[must_use]
    pub(crate) fn selections(&self) -> &[AgentChatSelection] {
        &self.selections
    }

    /// Drives every pre-approved host at most once.
    ///
    /// Hosts remain inactive until the post-commit prompt wake reaches their router, so this
    /// cadence cannot launch a provider before the durable prompt transaction completes.
    pub(crate) fn drive_once(&self) -> Result<(), OrdinaryAuthorityError> {
        self.router
            .lock()
            .map_err(|_| OrdinaryAuthorityError::RouterUnavailable)?
            .drive_once()
            .map_err(|_| OrdinaryAuthorityError::RouterUnavailable)
    }
}

/// Preflights and composes the private ordinary Claude/Codex lifecycle without bootstrap wiring.
///
/// Both evidence records are read and validated before either private resolver or sandboxed
/// runner is constructed. Each provider-specific composition repeats its preflight immediately
/// before construction, closing the read-to-compose interval without accepting a stale result.
///
/// # Errors
/// Returns before any private-prefix discovery or contained launch if either preflight fails.
pub(crate) fn compose_ordinary_authority<C, X>(
    state: &DaemonCompositionState,
    config: OrdinaryAuthorityConfig<C>,
    codex_sandbox: X,
) -> Result<OrdinaryAuthorityRuntime, OrdinaryAuthorityError>
where
    C: SandboxedProviderLaunch + std::fmt::Debug + 'static,
    X: SandboxedProviderLaunch + 'static,
{
    validate_shared_owner(&config)?;
    validate_selections(&config.selections)?;
    preflight_all(state, &config)?;
    let selections = config.selections.clone();
    let codex = compose_private_codex_authority(state, &config.codex, codex_sandbox)?;
    let claude = compose_private_claude_authority(state, config.claude)?;
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
    Ok(OrdinaryAuthorityRuntime {
        router: Arc::new(Mutex::new(router)),
        selections,
    })
}

fn validate_shared_owner<C>(
    config: &OrdinaryAuthorityConfig<C>,
) -> Result<(), OrdinaryAuthorityError> {
    (config.codex.coordinator_id == config.claude.coordinator_id)
        .then_some(())
        .ok_or(OrdinaryAuthorityError::CoordinatorMismatch)?;
    (config.codex.host_epoch == config.claude.host_epoch)
        .then_some(())
        .ok_or(OrdinaryAuthorityError::HostEpochMismatch)
}

fn validate_selections(selections: &[AgentChatSelection]) -> Result<(), OrdinaryAuthorityError> {
    (!selections.is_empty())
        .then_some(())
        .ok_or(OrdinaryAuthorityError::MissingSelections)?;
    for (index, selection) in selections.iter().enumerate() {
        selection
            .validate()
            .map_err(|_| OrdinaryAuthorityError::UnsupportedSelection)?;
        matches!(
            selection.provider,
            AgentChatProvider::Claude | AgentChatProvider::Codex
        )
        .then_some(())
        .ok_or(OrdinaryAuthorityError::UnsupportedSelection)?;
        matches!(selection.mode, AgentChatMode::Ask | AgentChatMode::Plan)
            .then_some(())
            .ok_or(OrdinaryAuthorityError::UnsupportedSelection)?;
        selections[..index]
            .iter()
            .all(|previous| previous != selection)
            .then_some(())
            .ok_or(OrdinaryAuthorityError::DuplicateSelection)?;
    }
    Ok(())
}

fn preflight_all<C>(
    state: &DaemonCompositionState,
    config: &OrdinaryAuthorityConfig<C>,
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
