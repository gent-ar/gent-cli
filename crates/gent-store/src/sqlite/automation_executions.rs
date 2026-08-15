//! `SQLite` persistence for durable automation execution records.

use gent_ports::{AutomationExecutionUpdate, LedgerError};
use gent_types::{AutomationExecutionPhase, AutomationExecutionRecord};
use rusqlite::{OptionalExtension, params};

use super::SqliteLedger;
use super::queries::storage_error;

pub(super) fn create(
    ledger: &SqliteLedger,
    execution: &AutomationExecutionRecord,
) -> Result<(), LedgerError> {
    validate(execution)?;
    ledger
        .lock()?
        .execute(
            "INSERT INTO automation_executions (execution_id, workspace_id, automation_id, trigger_key, phase) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                execution.execution_id,
                execution.workspace_id,
                execution.automation_id,
                execution.trigger_key,
                encode_phase(execution.phase)
            ],
        )
        .map(|_| ())
        .map_err(storage_error)
}

pub(super) fn find(
    ledger: &SqliteLedger,
    execution_id: &str,
) -> Result<Option<AutomationExecutionRecord>, LedgerError> {
    let connection = ledger.lock()?;
    find_connection(&connection, execution_id)
}

pub(super) fn list(
    ledger: &SqliteLedger,
    workspace_id: &str,
) -> Result<Vec<AutomationExecutionRecord>, LedgerError> {
    let connection = ledger.lock()?;
    let mut statement = connection
        .prepare(
            "SELECT execution_id, workspace_id, automation_id, trigger_key, phase FROM automation_executions WHERE workspace_id = ?1 ORDER BY rowid",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map([workspace_id], decode)
        .map_err(storage_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
}

pub(super) fn replace_phase(
    ledger: &SqliteLedger,
    execution_id: &str,
    expected: AutomationExecutionPhase,
    next: AutomationExecutionPhase,
) -> Result<AutomationExecutionUpdate, LedgerError> {
    let connection = ledger.lock()?;
    let current = find_connection(&connection, execution_id)?
        .ok_or_else(|| LedgerError::Invariant("automation execution does not exist".into()))?;
    if current.phase != expected {
        return Ok(AutomationExecutionUpdate::Current(current));
    }
    connection
        .execute(
            "UPDATE automation_executions SET phase = ?1 WHERE execution_id = ?2 AND phase = ?3",
            params![encode_phase(next), execution_id, encode_phase(expected)],
        )
        .map_err(storage_error)?;
    Ok(AutomationExecutionUpdate::Applied(
        AutomationExecutionRecord {
            phase: next,
            ..current
        },
    ))
}

fn validate(execution: &AutomationExecutionRecord) -> Result<(), LedgerError> {
    if execution.phase != AutomationExecutionPhase::Queued
        || [
            &execution.execution_id,
            &execution.workspace_id,
            &execution.automation_id,
            &execution.trigger_key,
        ]
        .into_iter()
        .any(|value| !valid_identity(value))
    {
        return Err(LedgerError::Invariant(
            "automation execution requires safe identities and queued phase".into(),
        ));
    }
    Ok(())
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
}

fn find_connection(
    connection: &rusqlite::Connection,
    execution_id: &str,
) -> Result<Option<AutomationExecutionRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT execution_id, workspace_id, automation_id, trigger_key, phase FROM automation_executions WHERE execution_id = ?1",
            [execution_id],
            decode,
        )
        .optional()
        .map_err(storage_error)
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutomationExecutionRecord> {
    Ok(AutomationExecutionRecord {
        execution_id: row.get(0)?,
        workspace_id: row.get(1)?,
        automation_id: row.get(2)?,
        trigger_key: row.get(3)?,
        phase: decode_phase(&row.get::<_, String>(4)?)?,
    })
}

const fn encode_phase(phase: AutomationExecutionPhase) -> &'static str {
    match phase {
        AutomationExecutionPhase::Queued => "queued",
        AutomationExecutionPhase::Running => "running",
        AutomationExecutionPhase::Succeeded => "succeeded",
        AutomationExecutionPhase::Failed => "failed",
        AutomationExecutionPhase::Interrupted => "interrupted",
    }
}

fn decode_phase(phase: &str) -> rusqlite::Result<AutomationExecutionPhase> {
    match phase {
        "queued" => Ok(AutomationExecutionPhase::Queued),
        "running" => Ok(AutomationExecutionPhase::Running),
        "succeeded" => Ok(AutomationExecutionPhase::Succeeded),
        "failed" => Ok(AutomationExecutionPhase::Failed),
        "interrupted" => Ok(AutomationExecutionPhase::Interrupted),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
