//! Private, read-only provider readiness decision for future approved authority.
//!
//! This module is deliberately absent from bootstrap and IPC. It neither starts Node nor npm:
//! callers receive a daemon-generated review before a separate receipt-bound install authority.

use gent_ports::{ProvisionedProviderLockReader, PublicProviderResolver};
use gent_protocol::DependencyProvider;
use gent_types::{AgentChatProvider, RunVersionLock};

use crate::locked_provider_resolver::LockedProviderResolver;

/// Read-only outcome for one selected public provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateProviderReadiness {
    /// A durable executable lock remains present and identity-checked.
    Ready(RunVersionLock),
    /// No durable installation exists; explicit review and consent are required.
    InstallReview,
    /// A stored installation changed or no longer satisfies its immutable lock.
    InvalidInstallation,
    /// Gent's public npm path never provisions the private Claurst bridge.
    ClaurstUnavailable,
    /// Durable installation provenance could not be read, so no install is proposed.
    Unavailable,
}

/// Produces daemon-owned readiness decisions without an external effect.
#[derive(Clone, Debug)]
pub(crate) struct PrivateProviderReadinessService<L> {
    installations: L,
}

impl<L> PrivateProviderReadinessService<L> {
    /// Binds the durable lock reader; this performs no provider discovery.
    #[must_use]
    pub(crate) const fn new(installations: L) -> Self {
        Self { installations }
    }
}

impl<L: Clone + ProvisionedProviderLockReader> PrivateProviderReadinessService<L> {
    /// Confirms durable lock state without generating a client-visible install artifact.
    pub(crate) fn assess(&self, provider: AgentChatProvider) -> PrivateProviderReadiness {
        let Some(provider) = public_provider(provider) else {
            return PrivateProviderReadiness::ClaurstUnavailable;
        };
        match self
            .installations
            .find_provisioned_provider_installation(provider.as_str())
        {
            Err(_) => PrivateProviderReadiness::Unavailable,
            Ok(None) => PrivateProviderReadiness::InstallReview,
            Ok(Some(_)) => LockedProviderResolver::new(self.installations.clone())
                .resolve(provider.as_str())
                .map_or(
                    PrivateProviderReadiness::InvalidInstallation,
                    PrivateProviderReadiness::Ready,
                ),
        }
    }
}

const fn public_provider(provider: AgentChatProvider) -> Option<DependencyProvider> {
    match provider {
        AgentChatProvider::Claude => Some(DependencyProvider::Claude),
        AgentChatProvider::Codex => Some(DependencyProvider::Codex),
        AgentChatProvider::Claurst => None,
    }
}

#[cfg(test)]
#[path = "private_provider_readiness_tests.rs"]
mod tests;
