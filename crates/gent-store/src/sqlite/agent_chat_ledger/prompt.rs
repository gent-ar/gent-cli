//! Atomic `SQLite` ownership for one persisted agent-chat prompt.

use gent_ports::{AgentChatPromptLedger, LedgerError};
use gent_types::{
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptOrigin, AgentChatPromptSaved,
    AgentChatRunId, Command, ConversationActivityFact, ConversationActivityScope,
    ConversationMessage, Receipt, ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::super::SqliteLedger;
use super::super::queries::{find_receipt, insert_receipt, receipt_matches_command, storage_error};
use super::prompt_dispatch::require_open;
#[path = "prompt_attachments.rs"]
mod prompt_attachments;
#[path = "prompt_retry.rs"]
mod retry;
use prompt_attachments::attach_available;
use retry::{existing, reject_receipt_id_collision};

const MAX_PROMPT_BYTES: usize = 64 * 1024;

impl AgentChatPromptLedger for SqliteLedger {
    fn save_agent_chat_prompt(
        &self,
        prompt: &AgentChatPromptCreate,
    ) -> Result<AgentChatPromptSaved, LedgerError> {
        save(self, prompt, None, &AgentChatPromptOrigin::User)
    }

    fn save_agent_chat_prompt_with_origin(
        &self,
        prompt: &AgentChatPromptCreate,
        origin: &AgentChatPromptOrigin,
    ) -> Result<AgentChatPromptSaved, LedgerError> {
        save(self, prompt, None, origin)
    }

    fn save_agent_chat_prompt_for_run(
        &self,
        prompt: &AgentChatPromptCreate,
        expected_run_id: &AgentChatRunId,
    ) -> Result<AgentChatPromptSaved, LedgerError> {
        save(
            self,
            prompt,
            Some(expected_run_id),
            &AgentChatPromptOrigin::User,
        )
    }
}

fn save(
    ledger: &SqliteLedger,
    prompt: &AgentChatPromptCreate,
    expected_run_id: Option<&AgentChatRunId>,
    origin: &AgentChatPromptOrigin,
) -> Result<AgentChatPromptSaved, LedgerError> {
    validate(prompt)?;
    let command = command_for(prompt);
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, prompt.host_epoch)?;
    if let Some(saved) = existing(&transaction, prompt)? {
        if !receipt_matches_command(&transaction, &command)? {
            return Err(LedgerError::Invariant(
                "agent chat prompt request is bound to another command".into(),
            ));
        }
        return Ok(saved);
    }
    let key = idempotency_key(prompt);
    if find_receipt(&transaction, &key)?.is_some() {
        return Err(LedgerError::Invariant(
            "agent chat prompt idempotency key is owned by another command".into(),
        ));
    }
    reject_receipt_id_collision(&transaction, prompt)?;
    let run_id = current_run(&transaction, &prompt.conversation_id.0)?;
    if expected_run_id.is_some_and(|expected| expected.0 != run_id) {
        return Err(LedgerError::Invariant(
            "agent chat prompt run is no longer the durable current run".into(),
        ));
    }
    let message = insert_prompt(&transaction, prompt, &run_id)?;
    attach_available(&transaction, &message.turn_id, &prompt.attachment_ids)?;
    transaction.execute("INSERT INTO agent_chat_transcript_events (conversation_id, cursor, event_id, turn_id, run_id, kind, text, is_partial, origin_json) VALUES (?1, (SELECT COALESCE(MAX(cursor), 0) + 1 FROM agent_chat_transcript_events WHERE conversation_id = ?1), ?2, ?3, ?4, 'userMessage', ?5, 0, ?6)", params![message.conversation_id, format!("user:{}", message.message_id), message.turn_id, message.run_id, message.text, serde_json::to_string(origin).map_err(storage_error)?]).map_err(storage_error)?;
    let receipt = Receipt {
        receipt_id: prompt.receipt_id.clone(),
        idempotency_key: key,
        status: ReceiptStatus::Settled,
        host_epoch: prompt.host_epoch,
    };
    insert_receipt(&transaction, &receipt, &command)?;
    transaction.execute("INSERT INTO agent_chat_prompt_receipts (request_id, idempotency_key, conversation_id, run_id, turn_id, message_id, disposition, tool_source_ids_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)", params![prompt.request_id.0, receipt.idempotency_key, prompt.conversation_id.0, run_id, message.turn_id, message.message_id, disposition(prompt.disposition), serde_json::to_string(&prompt.tool_source_ids).map_err(storage_error)?]).map_err(storage_error)?;
    transaction.execute("INSERT INTO agent_chat_prompt_dispatches (message_id, state, coordinator_id, host_epoch, created_rowid) VALUES (?1, 'awaiting_readiness', NULL, NULL, (SELECT COALESCE(MAX(created_rowid), 0) + 1 FROM agent_chat_prompt_dispatches))", params![message.message_id]).map_err(storage_error)?;
    if prompt.disposition == AgentChatPromptDisposition::Queue {
        super::super::agent_chat_queue_activity::append(
            &transaction,
            format!("agent-chat-queue:{}:queued", message.message_id),
            prompt.receipt_id.clone(),
            ConversationActivityFact::PromptQueued {
                scope: ConversationActivityScope {
                    conversation_id: message.conversation_id.clone(),
                    run_id: message.run_id.clone(),
                    turn_id: message.turn_id.clone(),
                    host_epoch: prompt.host_epoch,
                    cursor: 0,
                },
                message_id: message.message_id.clone(),
            },
        )?;
    }
    transaction.commit().map_err(storage_error)?;
    Ok(AgentChatPromptSaved {
        receipt,
        run_id: AgentChatRunId(run_id),
        message,
        disposition: prompt.disposition,
        delivery: prompt.disposition.delivery(),
        tool_source_ids: prompt.tool_source_ids.clone(),
    })
}

fn validate(prompt: &AgentChatPromptCreate) -> Result<(), LedgerError> {
    gent_types::validate_tool_source_ids(&prompt.tool_source_ids)
        .map_err(|error| LedgerError::Invariant(error.to_string()))?;
    if [
        &prompt.request_id.0,
        &prompt.receipt_id.0,
        &prompt.conversation_id.0,
    ]
    .into_iter()
    .any(|value| value.trim().is_empty())
        || (prompt.text.is_empty() && prompt.attachment_ids.is_empty())
        || prompt.text.len() > MAX_PROMPT_BYTES
        || prompt.text.contains('\0')
        || prompt
            .attachment_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 128 || id.chars().any(char::is_control))
        || {
            let mut ids = prompt.attachment_ids.clone();
            ids.sort_unstable();
            ids.windows(2).any(|pair| pair[0] == pair[1])
        }
    {
        return Err(LedgerError::Invariant(
            "agent chat prompt identity or text is invalid".into(),
        ));
    }
    Ok(())
}

fn current_run(
    transaction: &Transaction<'_>,
    conversation_id: &str,
) -> Result<String, LedgerError> {
    transaction
        .query_row(
            "SELECT 1 FROM agent_chat_conversations WHERE conversation_id = ?1",
            [conversation_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?
        .ok_or(LedgerError::Rejected(
            gent_types::AgentChatRejection::ConversationNotFound,
        ))?;
    transaction.query_row("SELECT current.run_id FROM agent_chat_conversations c JOIN agent_chat_run_selections current JOIN runs r ON r.run_id = current.run_id WHERE c.conversation_id = ?1 AND c.workspace_id IS NOT NULL AND r.conversation_id = c.conversation_id ORDER BY r.rowid DESC LIMIT 1", [conversation_id], |row| row.get(0)).optional().map_err(storage_error)?.ok_or_else(|| LedgerError::Invariant("agent chat conversation has no daemon-bound workspace and cannot accept a prompt".into()))
}

fn insert_prompt(
    transaction: &Transaction<'_>,
    prompt: &AgentChatPromptCreate,
    run_id: &str,
) -> Result<ConversationMessage, LedgerError> {
    let turn_id = stable_identity("turn", &prompt.request_id.0);
    let message_id = stable_identity("message", &prompt.request_id.0);
    let sequence: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM turns WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    let ordinal: i64 = transaction.query_row("SELECT COALESCE(MAX(ordinal), 0) + 1 FROM conversation_message_ordinals WHERE conversation_id = ?1", [&prompt.conversation_id.0], |row| row.get(0)).map_err(storage_error)?;
    let message = ConversationMessage {
        message_id,
        turn_id,
        conversation_id: prompt.conversation_id.0.clone(),
        run_id: run_id.into(),
        sequence: u64::try_from(sequence).map_err(storage_error)?,
        text: prompt.text.clone(),
        text_digest_sha256: digest(&prompt.text),
    };
    transaction.execute("INSERT INTO turns (turn_id, conversation_id, run_id, sequence, phase) VALUES (?1, ?2, ?3, ?4, 'active')", params![message.turn_id, message.conversation_id, message.run_id, sequence]).map_err(storage_error)?;
    transaction.execute("INSERT INTO conversation_messages (message_id, turn_id, conversation_id, run_id, text, text_digest_sha256, byte_len) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![message.message_id, message.turn_id, message.conversation_id, message.run_id, message.text, message.text_digest_sha256, message.text.len()]).map_err(storage_error)?;
    transaction.execute("INSERT INTO conversation_message_ordinals (message_id, conversation_id, ordinal) VALUES (?1, ?2, ?3)", params![message.message_id, message.conversation_id, ordinal]).map_err(storage_error)?;
    Ok(message)
}

fn command_for(prompt: &AgentChatPromptCreate) -> Command {
    Command {
        receipt_id: prompt.receipt_id.clone(),
        idempotency_key: idempotency_key(prompt),
        host_epoch: prompt.host_epoch,
        kind: "agentChatPrompt".into(),
        payload: json!({ "requestId": prompt.request_id, "conversationId": prompt.conversation_id, "disposition": prompt.disposition, "textDigestSha256": digest(&prompt.text), "textByteLen": prompt.text.len(), "attachmentIds": prompt.attachment_ids }),
    }
}

fn idempotency_key(prompt: &AgentChatPromptCreate) -> String {
    stable_identity("receipt", &prompt.request_id.0)
}

fn stable_identity(kind: &str, request_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-agent-chat-prompt-v1\0");
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(request_id.as_bytes());
    format!("agent-chat-{kind}-{:x}", digest.finalize())
}

fn digest(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

const fn disposition(value: AgentChatPromptDisposition) -> &'static str {
    match value {
        AgentChatPromptDisposition::Send => "send",
        AgentChatPromptDisposition::Queue => "queue",
    }
}
