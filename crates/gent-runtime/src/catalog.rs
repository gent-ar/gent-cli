//! Capability-catalog reconciliation is pure: declarations must match observed behavior.

use gent_ports::CapabilityCatalogLedger;
use gent_protocol::{
    CONVERSATION_STATUS_CAPABILITY, CONVERSATION_TIMELINE_CAPABILITY, EVENT_STREAM_CAPABILITY,
};
use gent_types::{CapabilityCatalogRecord, CapabilitySet};

use crate::Coordinator;
use crate::RuntimeError;

/// A capability the runtime may advertise after its transport proves a handler exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCapability {
    Decisions,
    EventResync,
    EventStream,
    Events,
    HostEpoch,
    Receipts,
}

impl RuntimeCapability {
    const fn wire_name(self) -> &'static str {
        match self {
            Self::Decisions => "decisions",
            Self::EventResync => "event-resync",
            Self::EventStream => EVENT_STREAM_CAPABILITY,
            Self::Events => "events",
            Self::HostEpoch => "host-epoch",
            Self::Receipts => "receipts",
        }
    }
}

const DECLARED: [RuntimeCapability; 6] = [
    RuntimeCapability::Decisions,
    RuntimeCapability::EventResync,
    RuntimeCapability::EventStream,
    RuntimeCapability::Events,
    RuntimeCapability::HostEpoch,
    RuntimeCapability::Receipts,
];

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum CatalogError {
    #[error("declared capability is not observed: {0}")]
    DeclaredButUnavailable(String),
    #[error("observed capability is absent from the catalog: {0}")]
    UndeclaredObserved(String),
}

/// Detects catalog drift before a daemon advertises capability availability.
///
/// # Errors
/// Returns an error for any declared-but-unavailable or observed-but-undeclared capability.
pub fn reconcile(declared: &CapabilitySet, observed: &CapabilitySet) -> Result<(), CatalogError> {
    for capability in &declared.0 {
        if !observed.0.contains(capability) {
            return Err(CatalogError::DeclaredButUnavailable(capability.clone()));
        }
    }
    for capability in &observed.0 {
        if !declared.0.contains(capability) {
            return Err(CatalogError::UndeclaredObserved(capability.clone()));
        }
    }
    Ok(())
}

/// Returns the one runtime-owned catalog eligible for wire advertisement.
#[must_use]
pub fn declared_capabilities() -> CapabilitySet {
    let mut capabilities = capability_set(DECLARED);
    capabilities
        .0
        .push(CONVERSATION_STATUS_CAPABILITY.to_owned());
    capabilities
        .0
        .push(CONVERSATION_TIMELINE_CAPABILITY.to_owned());
    capabilities
}

/// Converts typed handler observations into their stable wire representation.
#[must_use]
pub fn capability_set(observed: impl IntoIterator<Item = RuntimeCapability>) -> CapabilitySet {
    CapabilitySet(
        observed
            .into_iter()
            .map(|capability| capability.wire_name().into())
            .collect(),
    )
}

/// Rejects a transport whose observed handlers drift from the runtime catalog.
///
/// # Errors
/// Returns the mismatch before any status or handshake can advertise capabilities.
pub fn validate_observed_capabilities(
    observed: &CapabilitySet,
) -> Result<CapabilitySet, CatalogError> {
    let declared = declared_capabilities();
    reconcile(&declared, observed)?;
    Ok(declared)
}

impl<L> Coordinator<L>
where
    L: CapabilityCatalogLedger,
{
    /// Persists the complete capability set validated for this daemon process.
    ///
    /// # Errors
    /// Returns an error when durable storage rejects the catalog snapshot.
    pub fn persist_capability_catalog(&self) -> Result<(), RuntimeError> {
        self.ledger
            .save_capability_catalog(&CapabilityCatalogRecord {
                schema_version: 1,
                capabilities: self.capabilities.clone(),
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CatalogError, RuntimeCapability, capability_set, reconcile, validate_observed_capabilities,
    };
    use gent_types::CapabilitySet;

    #[test]
    fn declared_capability_drift_fails_the_build_gate() {
        assert_eq!(
            reconcile(
                &CapabilitySet(vec!["events".into()]),
                &CapabilitySet::default()
            ),
            Err(CatalogError::DeclaredButUnavailable("events".into()))
        );
    }

    #[test]
    fn exact_catalog_is_accepted() {
        let catalog = CapabilitySet(vec!["events".into(), "receipts".into()]);
        assert!(reconcile(&catalog, &catalog).is_ok());
    }

    #[test]
    fn typed_observations_cannot_add_an_undeclared_wire_capability() {
        let observed = capability_set([RuntimeCapability::Events]);
        assert_eq!(
            validate_observed_capabilities(&observed),
            Err(CatalogError::DeclaredButUnavailable("decisions".into()))
        );
    }
}
