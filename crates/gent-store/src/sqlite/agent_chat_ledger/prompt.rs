//! Atomic `SQLite` ownership for one persisted agent-chat prompt.

use gent_ports::{AgentChatPromptLedger, IngressMode, LedgerError};
use gent_types::{
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatRunId,
    Command, ConversationMessage, Receipt, ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::super::SqliteLedger;
use super::super::epoch::require_epoch;
use super::super::queries::{
    find_receipt, host_ingress, insert_receipt, receipt_matches_command, storage_error,
};

const MAX_PROMPT_BYTES: usize = 64 * 1024;

impl AgentChatPromptLedger for SqliteLedger {
    fn save_agent_chat_prompt(
        &self,
        prompt: &AgentChatPromptCreate,
    ) -> Result<AgentChatPromptSaved, LedgerError> {
        save(self, prompt, None)
    }

    fn save_agent_chat_prompt_for_run(
        &self,
        prompt: &AgentChatPromptCreate,
        expected_run_id: &AgentChatRunId,
    ) -> Result<AgentChatPromptSaved, LedgerError> {
        save(self, prompt, Some(expected_run_id))
    }
}

fn save(
    ledger: &SqliteLedger,
    prompt: &AgentChatPromptCreate,
    expected_run_id: Option<&AgentChatRunId>,
) -> Result<AgentChatPromptSaved, LedgerError> {
    validate(prompt)?;
    let command = command_for(prompt);
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(prompt.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
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
    let receipt = Receipt {
        receipt_id: prompt.receipt_id.clone(),
        idempotency_key: key,
        status: ReceiptStatus::Settled,
        host_epoch: prompt.host_epoch,
    };
    insert_receipt(&transaction, &receipt, &command)?;
    transaction.execute("INSERT INTO agent_chat_prompt_receipts (request_id, idempotency_key, conversation_id, run_id, turn_id, message_id, disposition) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![prompt.request_id.0, receipt.idempotency_key, prompt.conversation_id.0, run_id, message.turn_id, message.message_id, disposition(prompt.disposition)]).map_err(storage_error)?;
    if prompt.disposition == AgentChatPromptDisposition::Send {
        transaction.execute("INSERT INTO agent_chat_prompt_dispatches (message_id, state, coordinator_id, host_epoch, created_rowid) VALUES (?1, 'pending', NULL, NULL, (SELECT COALESCE(MAX(created_rowid), 0) + 1 FROM agent_chat_prompt_dispatches))", params![message.message_id]).map_err(storage_error)?;
    }
    transaction.commit().map_err(storage_error)?;
    Ok(AgentChatPromptSaved {
        receipt,
        run_id: AgentChatRunId(run_id),
        message,
        disposition: prompt.disposition,
        delivery: prompt.disposition.delivery(),
    })
}

fn validate(prompt: &AgentChatPromptCreate) -> Result<(), LedgerError> {
    if [
        &prompt.request_id.0,
        &prompt.receipt_id.0,
        &prompt.conversation_id.0,
    ]
    .into_iter()
    .any(|value| value.trim().is_empty())
        || prompt.text.is_empty()
        || prompt.text.len() > MAX_PROMPT_BYTES
        || prompt.text.contains('\0')
    {
        return Err(LedgerError::Invariant(
            "agent chat prompt identity or text is invalid".into(),
        ));
    }
    Ok(())
}

fn existing(
    transaction: &Transaction<'_>,
    prompt: &AgentChatPromptCreate,
) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
    let row = transaction.query_row("SELECT r.receipt_id, r.status, r.host_epoch, p.conversation_id, p.run_id, p.disposition, m.message_id, m.turn_id, t.sequence, m.text, m.text_digest_sha256 FROM agent_chat_prompt_receipts p JOIN receipts r ON r.idempotency_key = p.idempotency_key JOIN conversation_messages m ON m.message_id = p.message_id JOIN turns t ON t.turn_id = p.turn_id WHERE p.request_id = ?1", [&prompt.request_id.0], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, u64>(8)?, row.get::<_, String>(9)?, row.get::<_, String>(10)?))).optional().map_err(storage_error)?;
    let Some((
        receipt_id,
        status,
        epoch,
        conversation_id,
        run_id,
        saved_disposition,
        message_id,
        turn_id,
        sequence,
        text,
        digest,
    )) = row
    else {
        return Ok(None);
    };
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
    }))
}

fn reject_receipt_id_collision(
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

fn current_run(
    transaction: &Transaction<'_>,
    conversation_id: &str,
) -> Result<String, LedgerError> {
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
        payload: json!({ "requestId": prompt.request_id, "conversationId": prompt.conversation_id, "disposition": prompt.disposition, "textDigestSha256": digest(&prompt.text), "textByteLen": prompt.text.len() }),
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
