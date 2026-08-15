//! `SQLite` persistence for worktree-scoped Git operation records.

use gent_ports::{GitOperationUpdate, LedgerError};
use gent_types::{GitOperationKind, GitOperationPhase, GitOperationRecord};
use rusqlite::{OptionalExtension, params};

use super::SqliteLedger;
use super::queries::{find_run, storage_error};

pub(super) fn create(
    ledger: &SqliteLedger,
    operation: &GitOperationRecord,
) -> Result<(), LedgerError> {
    if operation.operation_id.is_empty()
        || operation.worktree_id.is_empty()
        || operation.run_id.is_empty()
        || operation.phase != GitOperationPhase::Requested
    {
        return Err(LedgerError::Invariant(
            "git operation requires identities and requested phase".into(),
        ));
    }
    let connection = ledger.lock()?;
    if find_run(&connection, &operation.run_id)?.is_none() {
        return Err(LedgerError::Invariant(
            "git operation run does not exist".into(),
        ));
    }
    connection.execute(
        "INSERT INTO git_operations (operation_id, worktree_id, run_id, kind, phase) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![operation.operation_id, operation.worktree_id, operation.run_id, encode_kind(&operation.kind), encode_phase(operation.phase)],
    ).map(|_| ()).map_err(storage_error)
}

pub(super) fn find(
    ledger: &SqliteLedger,
    operation_id: &str,
) -> Result<Option<GitOperationRecord>, LedgerError> {
    let connection = ledger.lock()?;
    find_connection(&connection, operation_id)
}

pub(super) fn replace_phase(
    ledger: &SqliteLedger,
    operation_id: &str,
    expected: GitOperationPhase,
    next: GitOperationPhase,
) -> Result<GitOperationUpdate, LedgerError> {
    let connection = ledger.lock()?;
    let current = find_connection(&connection, operation_id)?
        .ok_or_else(|| LedgerError::Invariant("git operation does not exist".into()))?;
    if current.phase != expected {
        return Ok(GitOperationUpdate::Current(current));
    }
    connection
        .execute(
            "UPDATE git_operations SET phase = ?1 WHERE operation_id = ?2 AND phase = ?3",
            params![encode_phase(next), operation_id, encode_phase(expected)],
        )
        .map_err(storage_error)?;
    Ok(GitOperationUpdate::Applied(GitOperationRecord {
        phase: next,
        ..current
    }))
}

fn find_connection(
    connection: &rusqlite::Connection,
    operation_id: &str,
) -> Result<Option<GitOperationRecord>, LedgerError> {
    connection.query_row("SELECT operation_id, worktree_id, run_id, kind, phase FROM git_operations WHERE operation_id = ?1", [operation_id], decode).optional().map_err(storage_error)
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<GitOperationRecord> {
    Ok(GitOperationRecord {
        operation_id: row.get(0)?,
        worktree_id: row.get(1)?,
        run_id: row.get(2)?,
        kind: decode_kind(&row.get::<_, String>(3)?)?,
        phase: decode_phase(&row.get::<_, String>(4)?)?,
    })
}

fn encode_kind(kind: &GitOperationKind) -> &'static str {
    match kind {
        GitOperationKind::Status => "status",
        GitOperationKind::Commit => "commit",
        GitOperationKind::CreateWorktree => "createWorktree",
        GitOperationKind::RemoveWorktree => "removeWorktree",
    }
}
fn decode_kind(kind: &str) -> rusqlite::Result<GitOperationKind> {
    match kind {
        "status" => Ok(GitOperationKind::Status),
        "commit" => Ok(GitOperationKind::Commit),
        "createWorktree" => Ok(GitOperationKind::CreateWorktree),
        "removeWorktree" => Ok(GitOperationKind::RemoveWorktree),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
const fn encode_phase(phase: GitOperationPhase) -> &'static str {
    match phase {
        GitOperationPhase::Requested => "requested",
        GitOperationPhase::Running => "running",
        GitOperationPhase::Succeeded => "succeeded",
        GitOperationPhase::Failed => "failed",
        GitOperationPhase::Interrupted => "interrupted",
    }
}
fn decode_phase(phase: &str) -> rusqlite::Result<GitOperationPhase> {
    match phase {
        "requested" => Ok(GitOperationPhase::Requested),
        "running" => Ok(GitOperationPhase::Running),
        "succeeded" => Ok(GitOperationPhase::Succeeded),
        "failed" => Ok(GitOperationPhase::Failed),
        "interrupted" => Ok(GitOperationPhase::Interrupted),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
