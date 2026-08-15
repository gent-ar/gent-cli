//! Capability-catalog reconciliation is pure: declarations must match observed behavior.

use gent_types::CapabilitySet;

/// A capability the runtime may advertise after its transport proves a handler exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeCapability {
    Decisions,
    EventResync,
    Events,
    HostEpoch,
    Receipts,
}

impl RuntimeCapability {
    const fn wire_name(self) -> &'static str {
        match self {
            Self::Decisions => "decisions",
            Self::EventResync => "event-resync",
            Self::Events => "events",
            Self::HostEpoch => "host-epoch",
            Self::Receipts => "receipts",
        }
    }
}

const DECLARED: [RuntimeCapability; 5] = [
    RuntimeCapability::Decisions,
    RuntimeCapability::EventResync,
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
    capability_set(DECLARED)
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
