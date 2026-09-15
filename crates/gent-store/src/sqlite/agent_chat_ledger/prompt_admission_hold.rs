use gent_ports::{LedgerError, PromptAdmission};
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, HostEpoch, PromptHoldReason,
    ProviderPromptReadinessFailureBinding, ReceiptId,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::super::SqliteLedger;
use super::super::queries::storage_error;
use super::prompt_dispatch::require_open;
use crate::sqlite::agent_chat_queue_activity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HoldExit {
    Released,
    Canceled,
}

#[derive(Clone, Debug)]
struct HeldPrompt {
    message: String,
    conversation: String,
    run: String,
    turn: String,
}

pub(super) fn hold(
    ledger: &SqliteLedger,
    prompt_receipt_id: &ReceiptId,
    host_epoch: HostEpoch,
    reason: PromptHoldReason,
) -> Result<(), LedgerError> {
    if prompt_receipt_id.0.trim().is_empty() {
        return Err(LedgerError::Invariant(
            "agent chat prompt admission hold identity is invalid".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let Some(prompt) = held_send_prompt(&transaction, prompt_receipt_id)? else {
        return Ok(());
    };
    if recorded(&transaction, &prompt, &["promptHeld"])? {
        return Ok(());
    }
    agent_chat_queue_activity::append(
        &transaction,
        format!("agent-chat-hold:{}", prompt.message),
        ReceiptId(format!("agentChatPromptHeld:{}", prompt.message)),
        ConversationActivityFact::PromptHeld {
            scope: scope(&prompt, host_epoch),
            message_id: prompt.message.clone(),
            receipt_id: prompt_receipt_id.0.clone(),
            reason,
        },
    )?;
    transaction.commit().map_err(storage_error)
}

pub(super) fn admission(
    ledger: &SqliteLedger,
    prompt_receipt_id: &ReceiptId,
) -> Result<PromptAdmission, LedgerError> {
    let state = ledger
        .lock()?
        .query_row(
            "SELECT d.state FROM agent_chat_prompt_receipts p JOIN receipts r ON r.idempotency_key = p.idempotency_key JOIN agent_chat_prompt_dispatches d ON d.message_id = p.message_id WHERE r.receipt_id = ?1",
            params![prompt_receipt_id.0],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?;
    Ok(match state.as_deref() {
        Some("awaiting_readiness") => PromptAdmission::Held,
        Some("provisioning") => PromptAdmission::Installing,
        _ => PromptAdmission::Admitted,
    })
}

pub(crate) fn clear(
    transaction: &Transaction<'_>,
    message_id: &str,
    host_epoch: HostEpoch,
    exit: HoldExit,
) -> Result<(), LedgerError> {
    let Some(prompt) = admitted_prompt(transaction, message_id)? else {
        return Ok(());
    };
    if !recorded(transaction, &prompt, &["promptHeld"])?
        || recorded(transaction, &prompt, &["promptReleased", "promptCanceled"])?
    {
        return Ok(());
    }
    let scope = scope(&prompt, host_epoch);
    let (suffix, fact) = match exit {
        HoldExit::Released => (
            "released",
            ConversationActivityFact::PromptReleased {
                scope,
                message_id: prompt.message.clone(),
            },
        ),
        HoldExit::Canceled => (
            "canceled",
            ConversationActivityFact::PromptCanceled {
                scope,
                message_id: prompt.message.clone(),
            },
        ),
    };
    agent_chat_queue_activity::append(
        transaction,
        format!("agent-chat-hold:{}:{suffix}", prompt.message),
        ReceiptId(format!("agentChatPromptHoldExit:{}", prompt.message)),
        fact,
    )
}

fn scope(prompt: &HeldPrompt, host_epoch: HostEpoch) -> ConversationActivityScope {
    ConversationActivityScope {
        conversation_id: prompt.conversation.clone(),
        run_id: prompt.run.clone(),
        turn_id: prompt.turn.clone(),
        host_epoch,
        cursor: 0,
    }
}

fn held_send_prompt(
    transaction: &Transaction<'_>,
    prompt_receipt_id: &ReceiptId,
) -> Result<Option<HeldPrompt>, LedgerError> {
    transaction
        .query_row(
            "SELECT m.message_id, m.conversation_id, m.run_id, m.turn_id FROM agent_chat_prompt_receipts p JOIN receipts r ON r.idempotency_key = p.idempotency_key JOIN conversation_messages m ON m.message_id = p.message_id JOIN agent_chat_prompt_dispatches d ON d.message_id = p.message_id WHERE r.receipt_id = ?1 AND p.disposition = 'send' AND d.state IN ('awaiting_readiness', 'provisioning')",
            params![prompt_receipt_id.0],
            decode,
        )
        .optional()
        .map_err(storage_error)
}

fn admitted_prompt(
    transaction: &Transaction<'_>,
    message_id: &str,
) -> Result<Option<HeldPrompt>, LedgerError> {
    transaction
        .query_row(
            "SELECT message_id, conversation_id, run_id, turn_id FROM conversation_messages WHERE message_id = ?1",
            params![message_id],
            decode,
        )
        .optional()
        .map_err(storage_error)
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<HeldPrompt> {
    Ok(HeldPrompt {
        message: row.get(0)?,
        conversation: row.get(1)?,
        run: row.get(2)?,
        turn: row.get(3)?,
    })
}

fn recorded(
    transaction: &Transaction<'_>,
    prompt: &HeldPrompt,
    kinds: &[&str],
) -> Result<bool, LedgerError> {
    let list = kinds
        .iter()
        .map(|kind| format!("'{kind}'"))
        .collect::<Vec<_>>()
        .join(", ");
    transaction
        .query_row(
            &format!(
                "SELECT 1 FROM conversation_activity_facts WHERE conversation_id = ?1 AND run_id = ?2 AND json_extract(payload, '$.type') IN ({list}) AND json_extract(payload, '$.messageId') = ?3 LIMIT 1"
            ),
            params![prompt.conversation, prompt.run, prompt.message],
            |_| Ok(true),
        )
        .optional()
        .map(|found| found.unwrap_or(false))
        .map_err(storage_error)
}

pub(super) fn note_exit(
    transaction: &Transaction<'_>,
    binding: &ProviderPromptReadinessFailureBinding,
    turn_id: &str,
) -> Result<(), LedgerError> {
    let cursor: u64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(cursor), 0) + 1 FROM agent_chat_transcript_events WHERE conversation_id = ?1",
            [&binding.conversation_id.0],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    let event_id = format!("provider-readiness-failed:{}", binding.prompt_receipt_id.0);
    let text = format!("{}: {}", binding.exit.notice(), binding.reason);
    transaction
        .execute(
            "INSERT INTO agent_chat_transcript_events (conversation_id, cursor, event_id, turn_id, run_id, kind, text, is_partial) VALUES (?1, ?2, ?3, ?4, ?5, 'notice', ?6, 0)",
            params![binding.conversation_id.0, cursor, event_id, turn_id, binding.run_id.0, text],
        )
        .map_err(storage_error)?;
    Ok(())
}
