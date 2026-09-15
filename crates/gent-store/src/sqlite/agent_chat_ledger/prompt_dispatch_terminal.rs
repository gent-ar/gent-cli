use gent_ports::LedgerError;
use gent_types::{DurableTurnPhase, HostEpoch};
use rusqlite::{TransactionBehavior, params};

use super::helpers::valid_owner;
use super::{SqliteLedger, require_open};
use crate::sqlite::{queries, turn_terminal};

pub(super) fn settle(
    ledger: &SqliteLedger,
    message_id: &str,
    coordinator_id: &str,
    host_epoch: HostEpoch,
    phase: DurableTurnPhase,
) -> Result<(), LedgerError> {
    valid_owner(coordinator_id)?;
    if message_id.trim().is_empty() || !phase.is_terminal() {
        return Err(LedgerError::Invariant(
            "agent chat terminal settlement is invalid".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(queries::storage_error)?;
    require_open(&transaction, host_epoch)?;
    let turn_id = turn_of(&transaction, message_id)?;
    let changed = transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'settled' WHERE message_id = ?1 AND state = 'started' AND coordinator_id = ?2 AND host_epoch = ?3",
            params![message_id, coordinator_id, host_epoch.0],
        )
        .map_err(queries::storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat dispatch is not owned by this coordinator".into(),
        ));
    }
    if !turn_terminal::settle(&transaction, &turn_id, host_epoch, phase)? {
        return Err(LedgerError::Invariant(
            "agent chat terminal turn is not active".into(),
        ));
    }
    transaction.commit().map_err(queries::storage_error)
}

pub(super) fn turn_of(
    transaction: &rusqlite::Transaction<'_>,
    message_id: &str,
) -> Result<String, LedgerError> {
    transaction
        .query_row(
            "SELECT turn_id FROM conversation_messages WHERE message_id = ?1",
            params![message_id],
            |row| row.get(0),
        )
        .map_err(queries::storage_error)
}

pub(super) fn abandon_unprovable(
    ledger: &SqliteLedger,
    message_id: &str,
    coordinator_id: &str,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    valid_owner(coordinator_id)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(queries::storage_error)?;
    require_open(&transaction, host_epoch)?;
    let changed = transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'unprovable' WHERE message_id = ?1 AND state = 'launching' AND coordinator_id = ?2 AND host_epoch = ?3",
            params![message_id, coordinator_id, host_epoch.0],
        )
        .map_err(queries::storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat dispatch is not owned by this coordinator".into(),
        ));
    }
    let turn_id = turn_of(&transaction, message_id)?;
    if !turn_terminal::settle(&transaction, &turn_id, host_epoch, DurableTurnPhase::Failed)? {
        return Err(LedgerError::Invariant(
            "agent chat unprovable turn is not active".into(),
        ));
    }
    transaction.commit().map_err(queries::storage_error)
}
