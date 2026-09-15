use gent_ports::{IngressMode, LedgerError};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, Command, HostEpoch, Receipt, ReceiptId, ReceiptStatus,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::SqliteLedger;
use super::epoch::require_epoch;
use super::queries::{find_receipt, host_ingress, insert_receipt, storage_error};
use super::turn_terminal::STEER_INTERRUPT_KEY_PREFIX;

pub(super) fn mark(
    ledger: &SqliteLedger,
    host_epoch: HostEpoch,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
) -> Result<bool, LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    let turn_id = transaction
        .query_row(
            "SELECT t.turn_id FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id JOIN turns t ON t.turn_id = m.turn_id WHERE m.conversation_id = ?1 AND m.run_id = ?2 AND d.state IN ('launching', 'started') AND t.phase IN ('active', 'waitingPermission', 'waitingQuestion') ORDER BY d.created_rowid LIMIT 1",
            params![conversation_id.0, run_id.0],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?;
    let Some(turn_id) = turn_id else {
        return Ok(false);
    };
    let key = format!("{STEER_INTERRUPT_KEY_PREFIX}{turn_id}");
    if find_receipt(&transaction, &key)?.is_none() {
        let receipt = Receipt {
            receipt_id: ReceiptId(format!("steer-interrupt:{turn_id}")),
            idempotency_key: key.clone(),
            status: ReceiptStatus::Settled,
            host_epoch,
        };
        let command = Command {
            receipt_id: receipt.receipt_id.clone(),
            idempotency_key: key,
            host_epoch,
            kind: "agentChatSteerInterrupt".into(),
            payload: serde_json::json!({ "conversationId": conversation_id.0, "runId": run_id.0, "turnId": turn_id }),
        };
        insert_receipt(&transaction, &receipt, &command)?;
    }
    transaction.commit().map_err(storage_error)?;
    Ok(true)
}
