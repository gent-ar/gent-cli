//! `SQLite` persistence for append-only, digest-addressed run checkpoints.

use gent_ports::LedgerError;
use gent_types::RunCheckpointRecord;
use rusqlite::{OptionalExtension, params};

use super::SqliteLedger;
use super::queries::{find_run, storage_error};

pub(super) fn save(
    ledger: &SqliteLedger,
    checkpoint: &RunCheckpointRecord,
) -> Result<(), LedgerError> {
    validate(checkpoint)?;
    let connection = ledger.lock()?;
    if find_run(&connection, &checkpoint.run_id)?.is_none() {
        return Err(LedgerError::Invariant(
            "checkpoint run does not exist".into(),
        ));
    }
    if let Some(previous) = latest(&connection, &checkpoint.run_id)? {
        let sequence = previous
            .sequence
            .checked_add(1)
            .ok_or_else(|| LedgerError::Invariant("checkpoint sequence overflow".into()))?;
        if checkpoint.sequence != sequence || checkpoint.event_cursor < previous.event_cursor {
            return Err(LedgerError::Invariant(
                "checkpoint sequence and event cursor must be monotonic".into(),
            ));
        }
    } else if checkpoint.sequence != 1 {
        return Err(LedgerError::Invariant(
            "first checkpoint sequence must be one".into(),
        ));
    }
    connection
        .execute(
            "INSERT INTO run_checkpoints (checkpoint_id, run_id, sequence, event_cursor, state_digest_sha256) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![checkpoint.checkpoint_id, checkpoint.run_id, checkpoint.sequence, checkpoint.event_cursor, checkpoint.state_digest_sha256],
        )
        .map(|_| ())
        .map_err(storage_error)
}

pub(super) fn list(
    ledger: &SqliteLedger,
    run_id: &str,
) -> Result<Vec<RunCheckpointRecord>, LedgerError> {
    let connection = ledger.lock()?;
    let mut statement = connection
        .prepare(
            "SELECT checkpoint_id, run_id, sequence, event_cursor, state_digest_sha256 FROM run_checkpoints WHERE run_id = ?1 ORDER BY sequence",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map([run_id], decode)
        .map_err(storage_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
}

fn latest(
    connection: &rusqlite::Connection,
    run_id: &str,
) -> Result<Option<RunCheckpointRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT checkpoint_id, run_id, sequence, event_cursor, state_digest_sha256 FROM run_checkpoints WHERE run_id = ?1 ORDER BY sequence DESC LIMIT 1",
            [run_id],
            decode,
        )
        .optional()
        .map_err(storage_error)
}

fn validate(checkpoint: &RunCheckpointRecord) -> Result<(), LedgerError> {
    if checkpoint.checkpoint_id.is_empty()
        || checkpoint.run_id.is_empty()
        || checkpoint.sequence == 0
        || !valid_digest(&checkpoint.state_digest_sha256)
    {
        return Err(LedgerError::Invariant(
            "checkpoint identity, run, sequence, and SHA-256 digest are required".into(),
        ));
    }
    Ok(())
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunCheckpointRecord> {
    Ok(RunCheckpointRecord {
        checkpoint_id: row.get(0)?,
        run_id: row.get(1)?,
        sequence: row.get(2)?,
        event_cursor: row.get(3)?,
        state_digest_sha256: row.get(4)?,
    })
}
