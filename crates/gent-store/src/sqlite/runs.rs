//! Immutable run-lineage persistence helpers.

use gent_ports::{LedgerError, RunRecord};
use rusqlite::params;

use super::SqliteLedger;
use super::queries::{find_run, storage_error};

/// Inserts one lineage node after verifying its optional parent exists.
pub(super) fn create(ledger: &SqliteLedger, run: &RunRecord) -> Result<(), LedgerError> {
    let connection = ledger.lock()?;
    if let Some(parent) = &run.parent_run_id {
        if find_run(&connection, parent)?.is_none() {
            return Err(LedgerError::Invariant("run parent does not exist".into()));
        }
    }
    connection
        .execute(
            "INSERT INTO runs (run_id, parent_run_id, provider) VALUES (?1, ?2, ?3)",
            params![run.run_id, run.parent_run_id, run.provider],
        )
        .map(|_| ())
        .map_err(storage_error)
}
