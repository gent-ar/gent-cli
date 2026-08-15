//! Durable capability-catalog persistence boundary.

use gent_types::CapabilityCatalogRecord;

use crate::LedgerError;

pub trait CapabilityCatalogLedger: Send + Sync {
    /// Stores a complete validated capability snapshot.
    ///
    /// # Errors
    /// Returns an error when persistence fails.
    fn save_capability_catalog(&self, catalog: &CapabilityCatalogRecord)
    -> Result<(), LedgerError>;
    /// Reads the latest complete snapshot.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn capability_catalog(&self) -> Result<Option<CapabilityCatalogRecord>, LedgerError>;
}
