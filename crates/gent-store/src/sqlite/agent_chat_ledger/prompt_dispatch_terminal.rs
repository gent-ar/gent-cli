use gent_ports::LedgerError;
use gent_types::{DurableTurnPhase, HostEpoch};
use rusqlite::{TransactionBehavior, params};

use super::helpers::valid_owner;
use super::{SqliteLedger, require_open};
use crate::sqlite::queries::storage_error;

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
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let changed = transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'settled' WHERE message_id = ?1 AND state = 'started' AND coordinator_id = ?2 AND host_epoch = ?3",
            params![message_id, coordinator_id, host_epoch.0],
        )
        .map_err(storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat dispatch is not owned by this coordinator".into(),
        ));
    }
    let changed = transaction
        .execute(
            "UPDATE turns SET phase = ?1 WHERE turn_id = (SELECT turn_id FROM conversation_messages WHERE message_id = ?2) AND phase IN ('active', 'waitingPermission', 'waitingQuestion')",
            params![phase_name(phase), message_id],
        )
        .map_err(storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat terminal turn is not active".into(),
        ));
    }
    transaction.commit().map_err(storage_error)
}

const fn phase_name(phase: DurableTurnPhase) -> &'static str {
    match phase {
        DurableTurnPhase::Completed => "completed",
        DurableTurnPhase::Interrupted => "interrupted",
        DurableTurnPhase::Failed => "failed",
        DurableTurnPhase::Active
        | DurableTurnPhase::WaitingPermission
        | DurableTurnPhase::WaitingQuestion => unreachable!(),
    }
}
