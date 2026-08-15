//! Durable run-lifecycle projection boundary implemented by the store.

use gent_types::RunProjectionRecord;

use crate::LedgerError;

/// Persistence for complete run-scoped reducer snapshots.
pub trait RunProjectionLedger: Send + Sync {
    /// Saves a strictly newer projection cursor, preserving duplicate idempotency.
    ///
    /// # Errors
    /// Returns an error when the run is absent, a cursor regresses, or persistence fails.
    fn save_run_projection(&self, record: &RunProjectionRecord) -> Result<(), LedgerError>;

    /// Reads the latest complete projection for a run.
    ///
    /// # Errors
    /// Returns an error when the projection cannot be read.
    fn find_run_projection(&self, run_id: &str)
    -> Result<Option<RunProjectionRecord>, LedgerError>;
}
