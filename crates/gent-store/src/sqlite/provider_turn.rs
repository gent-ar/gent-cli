use gent_ports::LedgerError;
use gent_types::{DurableTurnPhase, HostEpoch};
use rusqlite::{TransactionBehavior, params};

use super::{SqliteLedger, agent_chat_ledger::require_open, queries::storage_error};

pub(super) fn start(
    ledger: &SqliteLedger,
    conversation_id: &str,
    run_id: &str,
    host_epoch: HostEpoch,
) -> Result<String, LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let sequence = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM turns WHERE conversation_id = ?1 AND EXISTS (SELECT 1 FROM runs WHERE run_id = ?2 AND conversation_id = ?1)",
            params![conversation_id, run_id],
            |row| row.get::<_, u64>(0),
        )
        .map_err(storage_error)?;
    let turn_id = format!("{run_id}:provider:{sequence}");
    transaction
        .execute(
            "INSERT INTO turns (turn_id, conversation_id, run_id, sequence, phase) SELECT ?1, ?2, ?3, ?4, 'active' WHERE EXISTS (SELECT 1 FROM runs WHERE run_id = ?3 AND conversation_id = ?2)",
            params![turn_id, conversation_id, run_id, sequence],
        )
        .map_err(storage_error)?
        .eq(&1)
        .then_some(())
        .ok_or_else(|| LedgerError::Invariant("provider turn run is not in its conversation".into()))?;
    transaction.commit().map_err(storage_error)?;
    Ok(turn_id)
}

pub(super) fn settle(
    ledger: &SqliteLedger,
    turn_id: &str,
    host_epoch: HostEpoch,
    phase: DurableTurnPhase,
) -> Result<(), LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    if !super::turn_terminal::settle(&transaction, turn_id, host_epoch, phase)? {
        return Err(LedgerError::Invariant("provider turn is not active".into()));
    }
    transaction.commit().map_err(storage_error)
}
