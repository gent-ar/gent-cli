//! Adapter joining immutable run checkpoints to the public persistence port.

use gent_ports::{LedgerError, RunCheckpointLedger};
use gent_types::RunCheckpointRecord;

use super::{SqliteLedger, run_checkpoints};

impl RunCheckpointLedger for SqliteLedger {
    fn save_run_checkpoint(&self, checkpoint: &RunCheckpointRecord) -> Result<(), LedgerError> {
        run_checkpoints::save(self, checkpoint)
    }

    fn list_run_checkpoints(&self, run_id: &str) -> Result<Vec<RunCheckpointRecord>, LedgerError> {
        run_checkpoints::list(self, run_id)
    }
}
