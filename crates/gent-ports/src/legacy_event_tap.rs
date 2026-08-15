//! Read-only boundary for a future legacy-host lifecycle observer.

use gent_types::LegacyLifecycleTap;

use crate::PortError;

/// Supplies content-safe lifecycle facts strictly after a durable legacy cursor.
pub trait LegacyEventTap: Send + Sync {
    /// Reads observations without creating any Gent-side mutation.
    ///
    /// # Errors
    /// Returns an unavailable or adapter error without starting a process or writing a ledger.
    fn read_after(&self, cursor: u64) -> Result<Vec<LegacyLifecycleTap>, PortError>;
}
