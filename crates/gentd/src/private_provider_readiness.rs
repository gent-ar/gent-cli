//! Private, read-only provider readiness decision for future approved authority.
//!
//! This module is deliberately absent from bootstrap and IPC. It neither starts Node nor npm:
//! callers receive a daemon-generated review before a separate receipt-bound install authority.

use gent_ports::{ProvisionedProviderLockReader, PublicProviderResolver};
use gent_protocol::{DependencyAction, DependencyPlan, DependencyPlanRequest, DependencyProvider};
use gent_types::{AgentChatProvider, RunVersionLock};

use crate::{
    dependency_catalog::DependencyCatalog, locked_provider_resolver::LockedProviderResolver,
};

/// Read-only outcome for one selected public provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateProviderReadiness {
    /// A durable executable lock remains present and identity-checked.
    Ready(RunVersionLock),
    /// No durable installation exists; explicit review and consent are required.
    InstallReview(DependencyPlan),
    /// A stored installation changed or no longer satisfies its immutable lock.
    InvalidInstallation(DependencyPlan),
    /// Gent's public npm path never provisions the private Claurst bridge.
    ClaurstUnavailable,
    /// Durable installation provenance could not be read, so no install is proposed.
    Unavailable,
}

/// Produces daemon-owned readiness decisions without an external effect.
#[derive(Clone, Debug)]
pub(crate) struct PrivateProviderReadinessService<L> {
    catalog: DependencyCatalog,
    installations: L,
}

impl<L> PrivateProviderReadinessService<L> {
    /// Binds the read-only catalog and ledger port; this performs no provider discovery.
    #[must_use]
    pub(crate) const fn new(catalog: DependencyCatalog, installations: L) -> Self {
        Self {
            catalog,
            installations,
        }
    }
}

impl<L: Clone + ProvisionedProviderLockReader> PrivateProviderReadinessService<L> {
    /// Confirms a lock or generates the exact public install plan that a user must review.
    pub(crate) fn assess(&self, provider: AgentChatProvider) -> PrivateProviderReadiness {
        let Some(provider) = public_provider(provider) else {
            return PrivateProviderReadiness::ClaurstUnavailable;
        };
        let plan = self.catalog.plan(DependencyPlanRequest {
            provider,
            action: DependencyAction::Install,
        });
        match self
            .installations
            .find_provisioned_provider_installation(provider.as_str())
        {
            Err(_) => PrivateProviderReadiness::Unavailable,
            Ok(None) => PrivateProviderReadiness::InstallReview(plan),
            Ok(Some(_)) => LockedProviderResolver::new(self.installations.clone())
                .resolve(provider.as_str())
                .map_or(
                    PrivateProviderReadiness::InvalidInstallation(plan),
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
