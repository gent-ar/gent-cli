//! `SQLite` implementation of immutable conversation, run, and turn relationships.

use gent_ports::{LedgerError, RunRecord, TurnPhaseUpdate};
use gent_types::{ConversationRecord, DurableTurnPhase, TurnRecord};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::SqliteLedger;
use super::queries::{find_run, storage_error};

pub(super) fn create_conversation_run(
    ledger: &SqliteLedger,
    conversation: &ConversationRecord,
    run: &RunRecord,
) -> Result<(), LedgerError> {
    if run.parent_run_id.is_some() {
        return Err(LedgerError::Invariant(
            "conversation root run must name its conversation and have no parent".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    transaction
        .execute(
            "INSERT INTO conversations (conversation_id) VALUES (?1)",
            [&conversation.conversation_id],
        )
        .map_err(storage_error)?;
    insert_run(&transaction, run, Some(&conversation.conversation_id))?;
    transaction.commit().map_err(storage_error)
}

pub(super) fn find_conversation(
    ledger: &SqliteLedger,
    conversation_id: &str,
) -> Result<Option<ConversationRecord>, LedgerError> {
    let connection = ledger.lock()?;
    connection
        .query_row(
            "SELECT conversation_id FROM conversations WHERE conversation_id = ?1",
            [conversation_id],
            |row| {
                Ok(ConversationRecord {
                    conversation_id: row.get(0)?,
                })
            },
        )
        .optional()
        .map_err(storage_error)
}

pub(super) fn create_turn(ledger: &SqliteLedger, turn: &TurnRecord) -> Result<(), LedgerError> {
    let connection = ledger.lock()?;
    find_run(&connection, &turn.run_id)?
        .ok_or_else(|| LedgerError::Invariant("turn run does not exist".into()))?;
    if conversation_id_for_run(&connection, &turn.run_id)?.as_deref() != Some(&turn.conversation_id)
    {
        return Err(LedgerError::Invariant(
            "turn must belong to its run conversation".into(),
        ));
    }
    connection
        .execute(
            "INSERT INTO turns (turn_id, conversation_id, run_id, sequence, phase) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![turn.turn_id, turn.conversation_id, turn.run_id, turn.sequence, encode_phase(turn.phase)],
        )
        .map(|_| ())
        .map_err(storage_error)
}

pub(super) fn find_turn(
    ledger: &SqliteLedger,
    turn_id: &str,
) -> Result<Option<TurnRecord>, LedgerError> {
    let connection = ledger.lock()?;
    find_turn_by_id(&connection, turn_id)
}

pub(super) fn list_conversation_runs(
    ledger: &SqliteLedger,
    conversation_id: &str,
) -> Result<Vec<RunRecord>, LedgerError> {
    let connection = ledger.lock()?;
    let mut statement = connection
        .prepare("SELECT run_id, parent_run_id, provider FROM runs WHERE conversation_id = ?1 ORDER BY rowid")
        .map_err(storage_error)?;
    let rows = statement
        .query_map([conversation_id], decode_run)
        .map_err(storage_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
}

pub(super) fn list_run_turns(
    ledger: &SqliteLedger,
    run_id: &str,
) -> Result<Vec<TurnRecord>, LedgerError> {
    let connection = ledger.lock()?;
    read_turn(&connection, "WHERE run_id = ?1 ORDER BY sequence", [run_id])
}

pub(super) fn replace_turn_phase(
    ledger: &SqliteLedger,
    turn_id: &str,
    expected: DurableTurnPhase,
    next: DurableTurnPhase,
) -> Result<TurnPhaseUpdate, LedgerError> {
    let connection = ledger.lock()?;
    let current = find_turn_by_id(&connection, turn_id)?
        .ok_or_else(|| LedgerError::Invariant("turn does not exist".into()))?;
    if current.phase != expected {
        return Ok(TurnPhaseUpdate::Current(current));
    }
    connection
        .execute(
            "UPDATE turns SET phase = ?1 WHERE turn_id = ?2 AND phase = ?3",
            params![encode_phase(next), turn_id, encode_phase(expected)],
        )
        .map_err(storage_error)?;
    Ok(TurnPhaseUpdate::Applied(TurnRecord {
        phase: next,
        ..current
    }))
}

fn find_turn_by_id(
    connection: &rusqlite::Connection,
    turn_id: &str,
) -> Result<Option<TurnRecord>, LedgerError> {
    read_turn(connection, "WHERE turn_id = ?1", [turn_id]).map(|mut turns| turns.pop())
}

pub(super) fn insert_run(
    transaction: &Transaction<'_>,
    run: &RunRecord,
    conversation_id: Option<&str>,
) -> Result<(), LedgerError> {
    transaction
        .execute(
            "INSERT INTO runs (run_id, conversation_id, parent_run_id, provider) VALUES (?1, ?2, ?3, ?4)",
            params![run.run_id, conversation_id, run.parent_run_id, run.provider],
        )
        .map(|_| ())
        .map_err(storage_error)
}

pub(super) fn conversation_id_for_run(
    connection: &rusqlite::Connection,
    run_id: &str,
) -> Result<Option<String>, LedgerError> {
    connection
        .query_row(
            "SELECT conversation_id FROM runs WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(storage_error)
}

fn read_turn<P: rusqlite::Params>(
    connection: &rusqlite::Connection,
    clause: &str,
    values: P,
) -> Result<Vec<TurnRecord>, LedgerError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT turn_id, conversation_id, run_id, sequence, phase FROM turns {clause}"
        ))
        .map_err(storage_error)?;
    let rows = statement
        .query_map(values, decode_turn)
        .map_err(storage_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
}

fn decode_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord {
        run_id: row.get(0)?,
        parent_run_id: row.get(1)?,
        provider: row.get(2)?,
    })
}

fn decode_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnRecord> {
    Ok(TurnRecord {
        turn_id: row.get(0)?,
        conversation_id: row.get(1)?,
        run_id: row.get(2)?,
        sequence: row.get(3)?,
        phase: decode_phase(&row.get::<_, String>(4)?)?,
    })
}

const fn encode_phase(phase: DurableTurnPhase) -> &'static str {
    match phase {
        DurableTurnPhase::Active => "active",
        DurableTurnPhase::WaitingPermission => "waitingPermission",
        DurableTurnPhase::WaitingQuestion => "waitingQuestion",
        DurableTurnPhase::Completed => "completed",
        DurableTurnPhase::Interrupted => "interrupted",
        DurableTurnPhase::Failed => "failed",
    }
}

fn decode_phase(value: &str) -> rusqlite::Result<DurableTurnPhase> {
    match value {
        "active" => Ok(DurableTurnPhase::Active),
        "waitingPermission" => Ok(DurableTurnPhase::WaitingPermission),
        "waitingQuestion" => Ok(DurableTurnPhase::WaitingQuestion),
        "completed" => Ok(DurableTurnPhase::Completed),
        "interrupted" => Ok(DurableTurnPhase::Interrupted),
        "failed" => Ok(DurableTurnPhase::Failed),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
