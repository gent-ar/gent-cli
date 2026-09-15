//! Atomic `SQLite` persistence for a new conversation seeded from another's messages.

use gent_ports::{AgentChatForkLedger, LedgerError};
use gent_types::{
    AgentChatConversationId, AgentChatFork, AgentChatForked, AgentChatRunId, Command, Receipt,
    ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::super::SqliteLedger;
use super::super::queries::{find_receipt, insert_receipt, receipt_matches_command, storage_error};
use super::prompt_dispatch::require_open;

#[path = "fork_messages.rs"]
mod messages;
use messages::{copy_messages, insert_conversation, message_ordinal, source_messages};

impl AgentChatForkLedger for SqliteLedger {
    fn fork_agent_chat_conversation(
        &self,
        fork: &AgentChatFork,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<AgentChatForked, LedgerError> {
        persist(self, fork, conversation_id, run_id)
    }
}

fn persist(
    ledger: &SqliteLedger,
    fork: &AgentChatFork,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
) -> Result<AgentChatForked, LedgerError> {
    validate(fork, conversation_id, run_id)?;
    let idempotency_key = idempotency_key(fork);
    let command = command_for(fork, &idempotency_key, conversation_id, run_id);
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, fork.host_epoch)?;
    if let Some(result) = existing(&transaction, &idempotency_key, fork)? {
        if !receipt_matches_command(&transaction, &command)? {
            return Err(conflict());
        }
        return Ok(result);
    }
    if find_receipt(&transaction, &idempotency_key)?.is_some() {
        return Err(LedgerError::Invariant(
            "agent chat fork idempotency key is owned by another command".into(),
        ));
    }
    reject_receipt_id_collision(&transaction, fork)?;
    let (provider, model, effort, mode, workspace_id) =
        current_selection(&transaction, &fork.source_conversation_id.0)?;
    let boundary_ordinal = message_ordinal(
        &transaction,
        &fork.source_conversation_id.0,
        &fork.fork_through_message_id,
    )?;
    let messages = source_messages(
        &transaction,
        &fork.source_conversation_id.0,
        boundary_ordinal,
    )?;
    insert_conversation(
        &transaction,
        conversation_id,
        run_id,
        &provider,
        &model,
        &effort,
        &mode,
        workspace_id.as_deref(),
    )?;
    let copied = copy_messages(&transaction, conversation_id, run_id, &messages)?;
    let receipt = Receipt {
        receipt_id: fork.receipt_id.clone(),
        idempotency_key: idempotency_key.clone(),
        status: ReceiptStatus::Settled,
        host_epoch: fork.host_epoch,
    };
    insert_receipt(&transaction, &receipt, &command)?;
    transaction
        .execute(
            "INSERT INTO agent_chat_fork_receipts (idempotency_key, source_conversation_id, conversation_id, run_id, fork_through_message_id, context_through_ordinal) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                idempotency_key,
                fork.source_conversation_id.0,
                conversation_id.0,
                run_id.0,
                fork.fork_through_message_id,
                copied,
            ],
        )
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)?;
    Ok(AgentChatForked {
        receipt,
        source_conversation_id: fork.source_conversation_id.clone(),
        conversation_id: conversation_id.clone(),
        run_id: run_id.clone(),
    })
}

fn validate(
    fork: &AgentChatFork,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
) -> Result<(), LedgerError> {
    if [
        &fork.receipt_id.0,
        &fork.request_id.0,
        &fork.source_conversation_id.0,
        &fork.fork_through_message_id,
        &conversation_id.0,
        &run_id.0,
    ]
    .into_iter()
    .any(|value| value.trim().is_empty())
        || conversation_id.0 == fork.source_conversation_id.0
    {
        return Err(LedgerError::Invariant(
            "agent chat fork identities must be nonempty and the new conversation must differ from the source".into(),
        ));
    }
    Ok(())
}

fn idempotency_key(fork: &AgentChatFork) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-agent-chat-fork-v1\0receipt\0");
    digest.update(fork.request_id.0.as_bytes());
    format!("{:x}", digest.finalize())
}

fn existing(
    transaction: &Transaction<'_>,
    idempotency_key: &str,
    fork: &AgentChatFork,
) -> Result<Option<AgentChatForked>, LedgerError> {
    let row = transaction
        .query_row(
            "SELECT r.receipt_id, r.status, r.host_epoch, f.source_conversation_id, f.conversation_id, f.run_id, f.fork_through_message_id FROM agent_chat_fork_receipts f JOIN receipts r ON r.idempotency_key = f.idempotency_key WHERE f.idempotency_key = ?1",
            [idempotency_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    let Some((receipt_id, status, epoch, source, conversation, run, fork_point)) = row else {
        return Ok(None);
    };
    if receipt_id != fork.receipt_id.0
        || source != fork.source_conversation_id.0
        || fork_point != fork.fork_through_message_id
        || status != "settled"
    {
        return Err(conflict());
    }
    Ok(Some(AgentChatForked {
        receipt: Receipt {
            receipt_id: fork.receipt_id.clone(),
            idempotency_key: idempotency_key.to_owned(),
            status: ReceiptStatus::Settled,
            host_epoch: gent_types::HostEpoch(epoch),
        },
        source_conversation_id: fork.source_conversation_id.clone(),
        conversation_id: AgentChatConversationId(conversation),
        run_id: AgentChatRunId(run),
    }))
}

fn conflict() -> LedgerError {
    LedgerError::Invariant("agent chat fork retry conflicts with durable ownership".into())
}

fn reject_receipt_id_collision(
    transaction: &Transaction<'_>,
    fork: &AgentChatFork,
) -> Result<(), LedgerError> {
    let owner = transaction
        .query_row(
            "SELECT 1 FROM receipts WHERE receipt_id = ?1",
            [&fork.receipt_id.0],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?;
    owner.is_none().then_some(()).ok_or_else(|| {
        LedgerError::Invariant("agent chat receipt id is owned by another command".into())
    })
}

#[allow(clippy::type_complexity)]
pub(super) fn current_selection(
    transaction: &Transaction<'_>,
    source_conversation_id: &str,
) -> Result<(String, String, String, String, Option<String>), LedgerError> {
    transaction
        .query_row(
            "SELECT q.provider, q.model, q.effort, q.mode, c.workspace_id FROM agent_chat_conversations c JOIN agent_chat_run_selections q JOIN runs r ON r.run_id = q.run_id WHERE c.conversation_id = ?1 AND r.conversation_id = c.conversation_id ORDER BY r.rowid DESC LIMIT 1",
            [source_conversation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?
        .ok_or(LedgerError::Rejected(gent_types::AgentChatRejection::ConversationNotFound))
}

fn command_for(
    fork: &AgentChatFork,
    idempotency_key: &str,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
) -> Command {
    Command {
        receipt_id: fork.receipt_id.clone(),
        idempotency_key: idempotency_key.to_owned(),
        host_epoch: fork.host_epoch,
        kind: "agentChatForkConversation".into(),
        payload: json!({
            "sourceConversationId": fork.source_conversation_id.0,
            "forkThroughMessageId": fork.fork_through_message_id,
            "conversationId": conversation_id.0,
            "runId": run_id.0,
        }),
    }
}
