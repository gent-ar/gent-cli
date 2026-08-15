//! Coordinator calls for durable, secret-free run checkpoint metadata.

use gent_ports::RunCheckpointLedger;
use gent_types::RunCheckpointRecord;

use crate::{Coordinator, RuntimeError};

impl<L> Coordinator<L>
where
    L: gent_ports::Ledger + RunCheckpointLedger,
{
    /// Saves the next ordered checkpoint without resuming or launching a provider.
    ///
    /// # Errors
    /// Returns an error when run, sequence, cursor, or digest invariants fail.
    pub fn save_run_checkpoint(
        &self,
        checkpoint: &RunCheckpointRecord,
    ) -> Result<(), RuntimeError> {
        Ok(self.ledger.save_run_checkpoint(checkpoint)?)
    }

    /// Lists digest-addressed checkpoint metadata without exposing opaque checkpoint material.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn run_checkpoints(&self, run_id: &str) -> Result<Vec<RunCheckpointRecord>, RuntimeError> {
        Ok(self.ledger.list_run_checkpoints(run_id)?)
    }
}
