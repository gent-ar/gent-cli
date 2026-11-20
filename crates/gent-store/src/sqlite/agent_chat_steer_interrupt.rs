use gent_ports::LedgerError;
use gent_types::{
    AgentChatConversationId, AgentChatRunId, Command, HostEpoch, Receipt, ReceiptId, ReceiptStatus,
};
use rusqlite::Connection;

use super::queries::{find_receipt, insert_receipt};
use super::turn_terminal::STEER_INTERRUPT_KEY_PREFIX;

pub(super) fn record(
    db: &Connection,
    host_epoch: HostEpoch,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
    turn_id: &str,
) -> Result<(), LedgerError> {
    let key = format!("{STEER_INTERRUPT_KEY_PREFIX}{turn_id}");
    if find_receipt(db, &key)?.is_some() {
        return Ok(());
    }
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
    insert_receipt(db, &receipt, &command)
}
