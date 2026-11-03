use super::queries::{
    find_receipt, host_ingress, insert_receipt, receipt_matches_command, storage_error,
};
use super::{SqliteLedger, epoch::require_epoch};
use gent_ports::{IngressMode, LedgerError};
use gent_types::{AgentChatConversationId, Command, HostEpoch, Receipt, ReceiptId, ReceiptStatus};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub(super) fn queue_command(
    receipt_id: &ReceiptId,
    host_epoch: HostEpoch,
    conversation_id: &AgentChatConversationId,
    message_id: &str,
    idempotency_key: String,
    kind: &str,
) -> Result<Command, LedgerError> {
    if [
        receipt_id.0.as_str(),
        conversation_id.0.as_str(),
        message_id,
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(LedgerError::Invariant(
            "queued prompt command identity is invalid".into(),
        ));
    }
    Ok(Command {
        receipt_id: receipt_id.clone(),
        idempotency_key,
        host_epoch,
        kind: kind.into(),
        payload: serde_json::json!({ "conversationId": conversation_id.0, "messageId": message_id }),
    })
}

pub(super) fn receipted(
    ledger: &SqliteLedger,
    command: &Command,
    conflict: LedgerError,
    apply: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<(), LedgerError>,
) -> Result<Receipt, LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(command.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    if let Some(receipt) = find_receipt(&transaction, &command.idempotency_key)? {
        return receipt_matches_command(&transaction, command)?
            .then_some(receipt)
            .ok_or(conflict);
    }
    let receipt_owner = transaction
        .query_row(
            "SELECT 1 FROM receipts WHERE receipt_id = ?1",
            [&command.receipt_id.0],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?;
    if receipt_owner.is_some() {
        return Err(LedgerError::Invariant(
            "queued prompt receipt id is owned by another command".into(),
        ));
    }
    apply(&transaction)?;
    let receipt = Receipt {
        receipt_id: command.receipt_id.clone(),
        idempotency_key: command.idempotency_key.clone(),
        status: ReceiptStatus::Settled,
        host_epoch: command.host_epoch,
    };
    insert_receipt(&transaction, &receipt, command)?;
    transaction.commit().map_err(storage_error)?;
    Ok(receipt)
}

pub(super) fn queued_prompt(
    connection: &Connection,
    conversation_id: &AgentChatConversationId,
    message_id: &str,
) -> Result<Option<(String, String)>, LedgerError> {
    connection
        .query_row(
            "SELECT m.run_id, m.turn_id FROM agent_chat_prompt_dispatches d JOIN agent_chat_prompt_receipts p ON p.message_id = d.message_id JOIN conversation_messages m ON m.message_id = d.message_id WHERE d.message_id = ?1 AND m.conversation_id = ?2 AND p.disposition = 'queue' AND d.state IN ('awaiting_readiness', 'pending')",
            params![message_id, conversation_id.0],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(storage_error)
}
