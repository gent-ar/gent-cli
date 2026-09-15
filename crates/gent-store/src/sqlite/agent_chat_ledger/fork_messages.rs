use gent_ports::LedgerError;
use gent_types::{AgentChatConversationId, AgentChatRunId};
use rusqlite::{OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};

use super::super::super::queries::storage_error;

pub(super) fn message_ordinal(
    transaction: &Transaction<'_>,
    source_conversation_id: &str,
    message_id: &str,
) -> Result<u64, LedgerError> {
    transaction
        .query_row(
            "SELECT ordinal FROM conversation_message_ordinals WHERE conversation_id = ?1 AND message_id = ?2",
            params![source_conversation_id, message_id],
            |row| row.get::<_, u64>(0),
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| {
            LedgerError::Invariant(
                "agent chat fork point does not belong to the source conversation".into(),
            )
        })
}

pub(super) struct SourceMessage {
    text: String,
    text_digest_sha256: String,
}

pub(super) fn source_messages(
    transaction: &Transaction<'_>,
    source_conversation_id: &str,
    through_ordinal: u64,
) -> Result<Vec<SourceMessage>, LedgerError> {
    let mut statement = transaction
        .prepare(
            "SELECT m.text, m.text_digest_sha256 FROM conversation_messages m JOIN conversation_message_ordinals o ON o.message_id = m.message_id WHERE o.conversation_id = ?1 AND o.ordinal <= ?2 ORDER BY o.ordinal ASC",
        )
        .map_err(storage_error)?;
    statement
        .query_map(params![source_conversation_id, through_ordinal], |row| {
            Ok(SourceMessage {
                text: row.get(0)?,
                text_digest_sha256: row.get(1)?,
            })
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_conversation(
    transaction: &Transaction<'_>,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
    provider: &str,
    model: &str,
    effort: &str,
    mode: &str,
    workspace_id: Option<&str>,
) -> Result<(), LedgerError> {
    transaction
        .execute(
            "INSERT INTO conversations (conversation_id) VALUES (?1)",
            [&conversation_id.0],
        )
        .map_err(storage_error)?;
    transaction
        .execute(
            "INSERT INTO runs (run_id, conversation_id, parent_run_id, provider) VALUES (?1, ?2, NULL, ?3)",
            params![run_id.0, conversation_id.0, provider],
        )
        .map_err(storage_error)?;
    transaction.execute("INSERT INTO agent_chat_conversations (conversation_id, root_run_id, provider, model, effort, mode, workspace_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![conversation_id.0, run_id.0, provider, model, effort, mode, workspace_id]).map_err(storage_error)?;
    transaction.execute("INSERT INTO agent_chat_run_selections (run_id, provider, model, effort, mode) VALUES (?1, ?2, ?3, ?4, ?5)", params![run_id.0, provider, model, effort, mode]).map_err(storage_error)?;
    Ok(())
}

pub(super) fn copy_messages(
    transaction: &Transaction<'_>,
    conversation_id: &AgentChatConversationId,
    run_id: &AgentChatRunId,
    messages: &[SourceMessage],
) -> Result<u64, LedgerError> {
    for (index, message) in messages.iter().enumerate() {
        let sequence = u64::try_from(index + 1)
            .map_err(|_| LedgerError::Invariant("agent chat fork message count overflow".into()))?;
        let turn_id = copied_identity("turn", &conversation_id.0, sequence);
        let message_id = copied_identity("message", &conversation_id.0, sequence);
        transaction.execute("INSERT INTO turns (turn_id, conversation_id, run_id, sequence, phase) VALUES (?1, ?2, ?3, ?4, 'completed')", params![turn_id, conversation_id.0, run_id.0, sequence]).map_err(storage_error)?;
        transaction.execute("INSERT INTO conversation_messages (message_id, turn_id, conversation_id, run_id, text, text_digest_sha256, byte_len) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![message_id, turn_id, conversation_id.0, run_id.0, message.text, message.text_digest_sha256, message.text.len()]).map_err(storage_error)?;
        transaction.execute("INSERT INTO conversation_message_ordinals (message_id, conversation_id, ordinal) VALUES (?1, ?2, ?3)", params![message_id, conversation_id.0, sequence]).map_err(storage_error)?;
    }
    u64::try_from(messages.len())
        .map_err(|_| LedgerError::Invariant("agent chat fork message count overflow".into()))
}

fn copied_identity(kind: &str, conversation_id: &str, sequence: u64) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-agent-chat-fork-v1\0");
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(conversation_id.as_bytes());
    digest.update([0]);
    digest.update(sequence.to_le_bytes());
    format!("{kind}-{:x}", digest.finalize())
}
