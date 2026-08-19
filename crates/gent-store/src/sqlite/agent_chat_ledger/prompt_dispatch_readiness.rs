//! Atomic release of a prompt held until private provider readiness is proven.

use gent_ports::LedgerError;
use gent_types::{AgentChatRunId, HostEpoch};
use rusqlite::{TransactionBehavior, params};

use super::super::SqliteLedger;
use super::super::queries::storage_error;
use super::prompt_dispatch::require_open;

/// Promotes only the exact current selected run from held to claimable dispatch.
pub(super) fn release(
    ledger: &SqliteLedger,
    message_id: &str,
    expected_run_id: &AgentChatRunId,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    if message_id.trim().is_empty() || expected_run_id.0.trim().is_empty() {
        return Err(LedgerError::Invariant(
            "agent chat readiness release identity is invalid".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let changed = transaction.execute(
        "UPDATE agent_chat_prompt_dispatches SET state = 'pending' WHERE message_id = ?1 AND state = 'awaiting_readiness' AND EXISTS (SELECT 1 FROM conversation_messages m JOIN agent_chat_run_selections selected ON selected.run_id = m.run_id WHERE m.message_id = ?1 AND m.run_id = ?2 AND m.run_id = (SELECT current.run_id FROM runs current JOIN agent_chat_run_selections current_selected ON current_selected.run_id = current.run_id WHERE current.conversation_id = m.conversation_id ORDER BY current.rowid DESC LIMIT 1))",
        params![message_id, expected_run_id.0],
    ).map_err(storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat prompt is not held for the current reviewed run".into(),
        ));
    }
    transaction.commit().map_err(storage_error)
}
