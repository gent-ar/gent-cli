//! Sealed composition of the dormant ordinary Claude and Codex lifecycle hosts.
//!
//! This module deliberately has no bootstrap, argument, environment, or protocol surface. It
//! accepts one already-verified authority release, then creates bounded private hosts and shares
//! their router with a future facade composition. The returned cadence owner is the
//! only value that can drive that router.

use std::sync::{Arc, Mutex};

use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::AgentChatProvider;

use crate::claude_authority_composition::{
    PrivateClaudeAuthorityConfig, PrivateClaudeAuthorityError, compose_private_claude_authority,
};
use crate::codex_authority_composition::{
    PrivateCodexAuthorityConfig, PrivateCodexAuthorityError, compose_private_codex_authority,
};
use crate::node_runtime_lock::{AppNodeRuntimeLock, AppNodeRuntimeLockError};
use crate::ordinary_authority_release::{
    VerifiedOrdinaryAuthorityRelease, VerifiedProviderAuthority,
};
use crate::ordinary_lifecycle_cadence::{
    OrdinaryLifecycleCadence, OrdinaryPromptIngress, pair as cadence_pair,
};
use crate::ordinary_lifecycle_control::OrdinaryLifecycleControl;
use crate::ordinary_lifecycle_router::{
    OrdinaryLifecycleHost, OrdinaryProviderHost, OrdinaryPublicLifecycleRouter,
};
use crate::provider_lifecycle_host::ProviderLifecycleHost;
use crate::runtime_facade::DaemonCompositionState;

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;

/// Failure before or while assembling the dormant ordinary lifecycle router.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OrdinaryAuthorityError {
    #[error("ordinary authority requires at least one public provider")]
    MissingProvider,
    #[error("ordinary authority release does not match the daemon compatibility state")]
    CompatibilityMismatch,
    #[error(transparent)]
    Codex(#[from] PrivateCodexAuthorityError),
    #[error(transparent)]
    Claude(#[from] PrivateClaudeAuthorityError),
    #[error("ordinary lifecycle router is unavailable")]
    RouterUnavailable,
    #[error("daemon state is unavailable for ordinary authority ownership")]
    StateUnavailable,
    #[error(transparent)]
    AppNodeRuntime(#[from] AppNodeRuntimeLockError),
}

/// Caller-timed bounded cadence for one shared ordinary-provider router.
///
/// This contains no daemon task or timer. A later approved bootstrap must retain this value and
/// choose its cadence; the hard-observer bootstrap never constructs it.
#[derive(Clone, Debug)]
pub(crate) struct OrdinaryAuthorityRuntime {
    router: Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>>,
    control: OrdinaryLifecycleControl,
    prompt_ingress: OrdinaryPromptIngress<SqliteLedger>,
    cadence: OrdinaryLifecycleCadence<SqliteLedger>,
}

impl OrdinaryAuthorityRuntime {
    /// Returns the exact shared router intended for a future `RuntimeFacade` composition.
    #[must_use]
    pub(crate) fn router(&self) -> Arc<Mutex<OrdinaryPublicLifecycleRouter<SqliteLedger>>> {
        Arc::clone(&self.router)
    }

    /// Returns the sealed prompt admission and post-commit wake pair for the authority facade.
    #[must_use]
    pub(crate) fn prompt_ingress(&self) -> OrdinaryPromptIngress<SqliteLedger> {
        self.prompt_ingress.clone()
    }

    /// Returns the transient latch a future facade must check before committing a prompt.
    #[must_use]
    pub(crate) fn lifecycle_control(&self) -> OrdinaryLifecycleControl {
        self.control.clone()
    }

    /// Runs recovery and the demand-driven lifecycle cadence for this authority.
    pub(crate) async fn run_cadence(&self) -> Result<(), String> {
        self.cadence.clone().run().await
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

/// Composes the private ordinary Claude/Codex lifecycle from one verified release.
///
/// The caller has re-read and fully verified the single authority artifact against the locked
/// Node runtime immediately before this call. No evidence path, provider key, or package policy
/// is accepted here, so selected providers cannot be assembled from a second authority source.
///
/// # Errors
/// Returns before any private-prefix discovery or provider launch if either preflight fails.
pub(crate) fn compose_ordinary_authority(
    state: &DaemonCompositionState,
    release: &VerifiedOrdinaryAuthorityRelease,
    app_node: &AppNodeRuntimeLock,
) -> Result<OrdinaryAuthorityRuntime, OrdinaryAuthorityError> {
    validate(release, state)?;
    let (coordinator_id, host_epoch) = owner(state)?;
    let launcher = app_node.rechecked_read_only_launcher(STREAM_CAPTURE_BYTES)?;
    let mut hosts: Vec<Box<dyn OrdinaryLifecycleHost>> = Vec::new();
    for provider in release.providers() {
        match provider {
            VerifiedProviderAuthority::Codex(preflight) => {
                let host = compose_private_codex_authority(
                    state,
                    &PrivateCodexAuthorityConfig {
                        coordinator_id: coordinator_id.clone(),
                        host_epoch,
                    },
                    preflight,
                    launcher.clone(),
                )?;
                hosts.push(Box::new(OrdinaryProviderHost::new(
                    AgentChatProvider::Codex,
                    ProviderLifecycleHost::new(host),
                )));
            }
            VerifiedProviderAuthority::Claude(preflight) => {
                let host = compose_private_claude_authority(
                    state,
                    &PrivateClaudeAuthorityConfig {
                        coordinator_id: coordinator_id.clone(),
                        host_epoch,
                    },
                    preflight,
                    launcher.clone(),
                )?;
                hosts.push(Box::new(OrdinaryProviderHost::new(
                    AgentChatProvider::Claude,
                    ProviderLifecycleHost::new(host),
                )));
            }
        }
    }
    let router = OrdinaryPublicLifecycleRouter::new(
        AgentChatReadService::new(state.ledger().clone()),
        hosts,
    )
    .map_err(|_| OrdinaryAuthorityError::RouterUnavailable)?;
    let router = Arc::new(Mutex::new(router));
    let (control, prompt_ingress, cadence) = cadence_pair(Arc::clone(&router));
    Ok(OrdinaryAuthorityRuntime {
        router,
        control,
        prompt_ingress,
        cadence,
    })
}

fn validate(
    release: &VerifiedOrdinaryAuthorityRelease,
    state: &DaemonCompositionState,
) -> Result<(), OrdinaryAuthorityError> {
    (!release.providers().is_empty())
        .then_some(())
        .ok_or(OrdinaryAuthorityError::MissingProvider)?;
    (release.compatibility().manifest_sha256() == state.compatibility().manifest_sha256())
        .then_some(())
        .ok_or(OrdinaryAuthorityError::CompatibilityMismatch)
}

fn owner(
    state: &DaemonCompositionState,
) -> Result<(String, gent_types::HostEpoch), OrdinaryAuthorityError> {
    let epoch = state
        .coordinator()
        .status()
        .map_err(|_| OrdinaryAuthorityError::StateUnavailable)?
        .host_epoch;
    Ok((format!("gentd-{}", epoch.0), epoch))
}

#[cfg(test)]
#[path = "ordinary_authority_composition_tests.rs"]
mod tests;
