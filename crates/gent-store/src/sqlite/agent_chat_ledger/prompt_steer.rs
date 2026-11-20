use gent_ports::LedgerError;
use gent_types::{
    AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatRunId, ConversationActivityFact,
    ConversationActivityScope, HostEpoch,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::super::super::SqliteLedger;
use super::super::super::agent_chat_queue_activity::append;
use super::super::super::queries::storage_error;
use super::super::super::turn_terminal::steered;
use super::helpers::{saved, valid_owner};
use super::require_open;

pub(super) fn claim_for_turn(
    ledger: &SqliteLedger,
    coordinator_id: &str,
    host_epoch: HostEpoch,
    run_id: &AgentChatRunId,
    turn_id: &str,
) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
    valid_owner(coordinator_id)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    if steered(&transaction, turn_id)? {
        return Ok(None);
    }
    let oldest = transaction
        .query_row(
            "SELECT d.message_id, d.state FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id WHERE m.conversation_id = (SELECT conversation_id FROM runs WHERE run_id = ?1) AND d.state IN ('awaiting_readiness', 'provisioning', 'pending') AND NOT EXISTS (SELECT 1 FROM agent_chat_steer_targets s WHERE s.message_id = d.message_id AND s.state = 'rejected') ORDER BY d.created_rowid LIMIT 1",
            [&run_id.0],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(storage_error)?;
    let Some((message_id, state)) = oldest else {
        return Ok(None);
    };
    let mut prompt = saved(&transaction, &message_id)?;
    if state != "pending" || prompt.disposition != AgentChatPromptDisposition::Queue {
        return Ok(None);
    }
    if prompt.message.run_id != run_id.0 {
        super::super::switch_queue::move_prompt(
            &transaction,
            &message_id,
            &prompt.message.turn_id,
            &run_id.0,
        )?;
        prompt = saved(&transaction, &message_id)?;
    }
    transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'claimed', coordinator_id = ?1, host_epoch = ?2 WHERE message_id = ?3 AND state = 'pending'",
            params![coordinator_id, host_epoch.0, message_id],
        )
        .map_err(storage_error)?;
    transaction
        .execute(
            "INSERT INTO agent_chat_steer_targets(message_id, receipt_id, run_id, turn_id, state) VALUES (?1, ?2, ?3, ?4, 'claimed') ON CONFLICT(message_id) DO UPDATE SET receipt_id = excluded.receipt_id, run_id = excluded.run_id, turn_id = excluded.turn_id, state = 'claimed'",
            params![message_id, prompt.receipt.receipt_id.0, run_id.0, turn_id],
        )
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)?;
    Ok(Some(prompt))
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
    transaction
        .execute(
            "UPDATE agent_chat_steer_targets SET state = 'delivered' WHERE message_id = ?1",
            [message_id],
        )
        .map_err(storage_error)?;
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
    let receipt_id = prompt.receipt.receipt_id.clone();
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

#[cfg(test)]
#[path = "prompt_steer_tests.rs"]
mod tests;
