//! Durable boundary for append-only run checkpoints.

use gent_types::RunCheckpointRecord;

use crate::LedgerError;

/// Persistence boundary for ordered secret-free run checkpoints.
pub trait RunCheckpointLedger: Send + Sync {
    /// Saves the next checkpoint for an existing run.
    ///
    /// # Errors
    /// Returns an error when identity, sequence, cursor, or persistence invariants fail.
    fn save_run_checkpoint(&self, checkpoint: &RunCheckpointRecord) -> Result<(), LedgerError>;

    /// Lists a run's immutable checkpoints in sequence order.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn list_run_checkpoints(&self, run_id: &str) -> Result<Vec<RunCheckpointRecord>, LedgerError>;
}
