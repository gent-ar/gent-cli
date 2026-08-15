//! Immutable run-lineage persistence helpers.

use super::SqliteLedger;
use super::conversations::conversation_id_for_run;
use super::queries::{find_run, storage_error};
use gent_ports::{LedgerError, RunRecord};
use rusqlite::params;

/// Inserts one lineage node after verifying its optional parent exists.
pub(super) fn create(ledger: &SqliteLedger, run: &RunRecord) -> Result<(), LedgerError> {
    let connection = ledger.lock()?;
    let conversation_id = if let Some(parent) = &run.parent_run_id {
        find_run(&connection, parent)?
            .ok_or_else(|| LedgerError::Invariant("run parent does not exist".into()))?;
        conversation_id_for_run(&connection, parent)?
    } else {
        None
    };
    connection
        .execute(
            "INSERT INTO runs (run_id, conversation_id, parent_run_id, provider) VALUES (?1, ?2, ?3, ?4)",
            params![run.run_id, conversation_id, run.parent_run_id, run.provider],
        )
        .map(|_| ())
        .map_err(storage_error)
}
