//! Capability-catalog reconciliation is pure: declarations must match observed behavior.

use gent_types::CapabilitySet;

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

#[cfg(test)]
mod tests {
    use super::{CatalogError, reconcile};
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
}
