//! Sealed composition of the dormant ordinary Claude and Codex lifecycle hosts.
//!
//! This module deliberately has no bootstrap, argument, environment, or protocol surface. It
//! first validates every selected provider evidence input, then creates the already bounded private hosts
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
    OrdinaryLifecycleCadence, OrdinaryPromptIngress, pair as cadence_pair,
};
use crate::ordinary_lifecycle_control::OrdinaryLifecycleControl;
use crate::ordinary_lifecycle_router::{
    OrdinaryLifecycleHost, OrdinaryProviderHost, OrdinaryPublicLifecycleRouter,
};
use crate::provider_lifecycle_host::ProviderLifecycleHost;
use crate::runtime_facade::DaemonCompositionState;

const STREAM_CAPTURE_BYTES: usize = 64 * 1024;

/// One public provider authority included in a private ordinary composition.
///
/// A provider is independently evidence-gated. Requiring unavailable Claude evidence before an
/// otherwise valid Codex authority would make provider selection an accidental cross-provider
/// dependency and prevent a truthful partial rollout.
#[derive(Debug)]
pub(crate) enum OrdinaryProviderAuthorityConfig {
    Claude(PrivateClaudeAuthorityConfig),
    Codex(PrivateCodexAuthorityConfig),
}

/// Private inputs for one or more independently approved public-provider hosts.
///
/// Neither configuration is serializable or accepted by the shipped daemon. When multiple hosts
/// are composed, their coordinator and epoch must match so one router cannot operate two owners.
#[derive(Debug)]
pub(crate) struct OrdinaryAuthorityConfig {
    pub(crate) providers: Vec<OrdinaryProviderAuthorityConfig>,
}

/// Failure before or while assembling the dormant ordinary lifecycle router.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OrdinaryAuthorityError {
    #[error("ordinary Claude and Codex coordinators must match")]
    CoordinatorMismatch,
    #[error("ordinary Claude and Codex host epochs must match")]
    HostEpochMismatch,
    #[error("ordinary authority requires at least one public provider")]
    MissingProvider,
    #[error("ordinary authority contains a duplicate public provider")]
    DuplicateProvider,
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
    let mut hosts: Vec<Box<dyn OrdinaryLifecycleHost>> = Vec::new();
    for provider in config.providers {
        match provider {
            OrdinaryProviderAuthorityConfig::Codex(config) => {
                let host = compose_private_codex_authority(state, &config, launcher.clone())?;
                hosts.push(Box::new(OrdinaryProviderHost::new(
                    AgentChatProvider::Codex,
                    ProviderLifecycleHost::new(host),
                )));
            }
            OrdinaryProviderAuthorityConfig::Claude(config) => {
                let host = compose_private_claude_authority(state, config, launcher.clone())?;
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

fn validate_shared_owner(config: &OrdinaryAuthorityConfig) -> Result<(), OrdinaryAuthorityError> {
    let Some(first) = config.providers.first() else {
        return Err(OrdinaryAuthorityError::MissingProvider);
    };
    let (first_owner, epoch) = owner(first);
    for (index, provider) in config.providers.iter().enumerate() {
        let (candidate_owner, candidate_epoch) = owner(provider);
        if candidate_owner != first_owner {
            return Err(OrdinaryAuthorityError::CoordinatorMismatch);
        }
        if candidate_epoch != epoch {
            return Err(OrdinaryAuthorityError::HostEpochMismatch);
        }
        if config.providers[..index]
            .iter()
            .any(|previous| provider_name(previous) == provider_name(provider))
        {
            return Err(OrdinaryAuthorityError::DuplicateProvider);
        }
    }
    Ok(())
}

fn preflight_all(
    state: &DaemonCompositionState,
    config: &OrdinaryAuthorityConfig,
) -> Result<(), OrdinaryAuthorityError> {
    for provider in &config.providers {
        match provider {
            OrdinaryProviderAuthorityConfig::Codex(config) => {
                codex_authority_preflight::load(
                    &config.evidence_record,
                    &config.trusted_keys,
                    state.compatibility(),
                    config.now_unix_seconds,
                )?;
            }
            OrdinaryProviderAuthorityConfig::Claude(config) => {
                claude_authority_preflight::load(
                    &config.evidence_record,
                    &config.trusted_keys,
                    state.compatibility(),
                    config.now_unix_seconds,
                )?;
            }
        }
    }
    Ok(())
}

fn owner(provider: &OrdinaryProviderAuthorityConfig) -> (&str, gent_types::HostEpoch) {
    match provider {
        OrdinaryProviderAuthorityConfig::Claude(config) => {
            (&config.coordinator_id, config.host_epoch)
        }
        OrdinaryProviderAuthorityConfig::Codex(config) => {
            (&config.coordinator_id, config.host_epoch)
        }
    }
}

const fn provider_name(provider: &OrdinaryProviderAuthorityConfig) -> AgentChatProvider {
    match provider {
        OrdinaryProviderAuthorityConfig::Claude(_) => AgentChatProvider::Claude,
        OrdinaryProviderAuthorityConfig::Codex(_) => AgentChatProvider::Codex,
    }
}

#[cfg(test)]
#[path = "ordinary_authority_composition_tests.rs"]
mod tests;
