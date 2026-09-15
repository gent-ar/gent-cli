use gent_ports::LedgerError;
use gent_types::{
    AgentChatPromptCreate, AgentChatPromptSaved, AgentChatRunId, ConversationMessage, Receipt,
    ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction};

use super::super::super::queries::storage_error;
use super::{disposition, idempotency_key};

pub(super) fn existing(
    transaction: &Transaction<'_>,
    prompt: &AgentChatPromptCreate,
) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
    let row = transaction.query_row("SELECT r.receipt_id, r.status, r.host_epoch, p.conversation_id, p.run_id, p.disposition, p.tool_source_ids_json, m.message_id, m.turn_id, t.sequence, m.text, m.text_digest_sha256 FROM agent_chat_prompt_receipts p JOIN receipts r ON r.idempotency_key = p.idempotency_key JOIN conversation_messages m ON m.message_id = p.message_id JOIN turns t ON t.turn_id = p.turn_id WHERE p.request_id = ?1", [&prompt.request_id.0], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?, row.get::<_, u64>(9)?, row.get::<_, String>(10)?, row.get::<_, String>(11)?))).optional().map_err(storage_error)?;
    let Some((
        receipt_id,
        status,
        epoch,
        conversation_id,
        run_id,
        saved_disposition,
        tool_source_ids_json,
        message_id,
        turn_id,
        sequence,
        text,
        digest,
    )) = row
    else {
        return Ok(None);
    };
    let tool_source_ids = serde_json::from_str(&tool_source_ids_json)
        .map_err(|_| LedgerError::Invariant("stored tool-source selection is invalid".into()))?;
    if receipt_id != prompt.receipt_id.0
        || conversation_id != prompt.conversation_id.0
        || saved_disposition != disposition(prompt.disposition)
        || text != prompt.text
    {
        return Err(LedgerError::Invariant(
            "agent chat prompt retry conflicts with durable ownership".into(),
        ));
    }
    if status != "settled" {
        return Err(LedgerError::Invariant(
            "agent chat prompt receipt must settle in its write transaction".into(),
        ));
    }
    Ok(Some(AgentChatPromptSaved {
        receipt: Receipt {
            receipt_id: prompt.receipt_id.clone(),
            idempotency_key: idempotency_key(prompt),
            status: ReceiptStatus::Settled,
            host_epoch: gent_types::HostEpoch(epoch),
        },
        run_id: AgentChatRunId(run_id.clone()),
        message: ConversationMessage {
            message_id,
            turn_id,
            conversation_id,
            run_id,
            sequence,
            text,
            text_digest_sha256: digest,
        },
        disposition: prompt.disposition,
        delivery: prompt.disposition.delivery(),
        tool_source_ids,
    }))
}

pub(super) fn reject_receipt_id_collision(
    transaction: &Transaction<'_>,
    prompt: &AgentChatPromptCreate,
) -> Result<(), LedgerError> {
    let owner = transaction
        .query_row(
            "SELECT 1 FROM receipts WHERE receipt_id = ?1",
            [&prompt.receipt_id.0],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?;
    if owner.is_some() {
        return Err(LedgerError::Invariant(
            "agent chat prompt receipt id is owned by another command".into(),
        ));
    }
    Ok(())
}
