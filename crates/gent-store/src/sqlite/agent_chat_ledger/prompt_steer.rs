use gent_ports::LedgerError;
use gent_types::{
    AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatRunId, ConversationActivityFact,
    ConversationActivityScope, HostEpoch, ReceiptId,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::super::super::SqliteLedger;
use super::super::super::agent_chat_queue_activity::{STEER_KEY_PREFIX, append};
use super::super::super::queries::storage_error;
use super::helpers::{saved, valid_owner};
use super::require_open;

pub(super) fn claim(
    ledger: &SqliteLedger,
    coordinator_id: &str,
    host_epoch: HostEpoch,
    run_id: &AgentChatRunId,
) -> Result<Option<(AgentChatPromptSaved, ReceiptId)>, LedgerError> {
    valid_owner(coordinator_id)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let oldest = transaction
        .query_row(
            "SELECT d.message_id, d.state, (SELECT r.receipt_id FROM receipts r WHERE r.idempotency_key = ?2 || d.message_id) FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id WHERE m.run_id = ?1 AND d.state IN ('awaiting_readiness', 'provisioning', 'pending') ORDER BY d.created_rowid LIMIT 1",
            params![run_id.0, STEER_KEY_PREFIX],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)),
        )
        .optional()
        .map_err(storage_error)?;
    let Some((message_id, state, Some(steer_receipt_id))) = oldest else {
        return Ok(None);
    };
    let prompt = saved(&transaction, &message_id)?;
    if state != "pending" || prompt.disposition != AgentChatPromptDisposition::Queue {
        return Ok(None);
    }
    transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'launching', coordinator_id = ?1, host_epoch = ?2 WHERE message_id = ?3 AND state = 'pending'",
            params![coordinator_id, host_epoch.0, message_id],
        )
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)?;
    Ok(Some((prompt, ReceiptId(steer_receipt_id))))
}

pub(super) fn deliver(
    ledger: &SqliteLedger,
    message_id: &str,
    coordinator_id: &str,
    host_epoch: HostEpoch,
    turn_id: &str,
) -> Result<(), LedgerError> {
    settle(
        ledger,
        message_id,
        coordinator_id,
        host_epoch,
        Some(turn_id),
    )
}

pub(super) fn start_turn(
    ledger: &SqliteLedger,
    message_id: &str,
    coordinator_id: &str,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    settle(ledger, message_id, coordinator_id, host_epoch, None)
}

fn settle(
    ledger: &SqliteLedger,
    message_id: &str,
    coordinator_id: &str,
    host_epoch: HostEpoch,
    into_turn_id: Option<&str>,
) -> Result<(), LedgerError> {
    valid_owner(coordinator_id)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let prompt = saved(&transaction, message_id)?;
    let changed = transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = ?1 WHERE message_id = ?2 AND state = 'launching' AND coordinator_id = ?3 AND host_epoch = ?4",
            params![into_turn_id.map_or("started", |_| "settled"), message_id, coordinator_id, host_epoch.0],
        )
        .map_err(storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat steered prompt is not owned by this coordinator".into(),
        ));
    }
    let scope = |turn_id: &str| ConversationActivityScope {
        conversation_id: prompt.message.conversation_id.clone(),
        run_id: prompt.message.run_id.clone(),
        turn_id: turn_id.into(),
        host_epoch,
        cursor: 0,
    };
    let Some(turn_id) = into_turn_id else {
        append(
            &transaction,
            format!("agent-chat-queue:{message_id}:released"),
            prompt.receipt.receipt_id.clone(),
            ConversationActivityFact::PromptReleased {
                scope: scope(&prompt.message.turn_id),
                message_id: message_id.into(),
            },
        )?;
        return transaction.commit().map_err(storage_error);
    };
    retire_queued_turn(&transaction, &prompt, turn_id)?;
    let receipt_id = steer_receipt(&transaction, message_id)?;
    let transcript_cursor = transaction
        .query_row(
            "SELECT COALESCE(MAX(cursor), 0) FROM agent_chat_transcript_events WHERE conversation_id = ?1",
            [&prompt.message.conversation_id],
            |row| row.get::<_, u64>(0),
        )
        .map_err(storage_error)?;
    append(
        &transaction,
        format!("agent-chat-queue:{message_id}:steered"),
        receipt_id.clone(),
        ConversationActivityFact::PromptSteered {
            scope: scope(turn_id),
            message_id: message_id.into(),
            receipt_id: receipt_id.0,
            transcript_cursor,
        },
    )?;
    transaction.commit().map_err(storage_error)
}

fn retire_queued_turn(
    transaction: &Transaction<'_>,
    prompt: &AgentChatPromptSaved,
    turn_id: &str,
) -> Result<(), LedgerError> {
    let target_is_live = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM turns WHERE turn_id = ?1 AND run_id = ?2 AND turn_id <> ?3)",
            params![turn_id, prompt.message.run_id, prompt.message.turn_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(storage_error)?;
    let retired = transaction
        .execute(
            "UPDATE turns SET phase = 'completed' WHERE turn_id = ?1 AND phase = 'active'",
            [&prompt.message.turn_id],
        )
        .map_err(storage_error)?;
    (target_is_live && retired == 1)
        .then_some(())
        .ok_or_else(|| {
            LedgerError::Invariant("steered prompt target turn is not in its run".into())
        })
}

fn steer_receipt(
    transaction: &Transaction<'_>,
    message_id: &str,
) -> Result<ReceiptId, LedgerError> {
    transaction
        .query_row(
            "SELECT receipt_id FROM receipts WHERE idempotency_key = ?1",
            [format!("{STEER_KEY_PREFIX}{message_id}")],
            |row| row.get::<_, String>(0),
        )
        .map(ReceiptId)
        .map_err(storage_error)
}
