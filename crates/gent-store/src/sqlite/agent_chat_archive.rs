use gent_ports::{AgentChatArchiveLedger, LedgerError};
use gent_types::{
    AgentChatConversationId, AgentChatRejection, Command, HostEpoch, Receipt, ReceiptId,
};
use rusqlite::OptionalExtension;

use super::SqliteLedger;
use super::agent_chat_queue_commands::receipted;
use super::queries::storage_error;

impl AgentChatArchiveLedger for SqliteLedger {
    fn archive_agent_chat_conversation(
        &self,
        receipt_id: &ReceiptId,
        host_epoch: HostEpoch,
        conversation_id: &AgentChatConversationId,
    ) -> Result<Receipt, LedgerError> {
        if receipt_id.0.trim().is_empty() || conversation_id.0.trim().is_empty() {
            return Err(LedgerError::Invariant(
                "conversation archive identity is invalid".into(),
            ));
        }
        let command = Command {
            receipt_id: receipt_id.clone(),
            idempotency_key: format!("agent-chat-archive-conversation:{}", receipt_id.0),
            host_epoch,
            kind: "agentChatArchiveConversation".into(),
            payload: serde_json::json!({ "conversationId": conversation_id.0 }),
        };
        let conflict = LedgerError::Invariant(
            "conversation archive receipt is bound to another conversation".into(),
        );
        receipted(self, &command, conflict, |transaction| {
            transaction
                .query_row(
                    "SELECT 1 FROM agent_chat_conversations WHERE conversation_id = ?1",
                    [&conversation_id.0],
                    |_| Ok(()),
                )
                .optional()
                .map_err(storage_error)?
                .ok_or(LedgerError::Rejected(
                    AgentChatRejection::ConversationNotFound,
                ))?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO archived_conversations (conversation_id, receipt_id, archived_at_unix_ms) VALUES (?1, ?2, CAST(unixepoch('subsec') * 1000 AS INTEGER))",
                    [&conversation_id.0, &receipt_id.0],
                )
                .map(|_| ())
                .map_err(storage_error)
        })
    }
}

pub(super) fn is_archived(
    connection: &rusqlite::Connection,
    conversation_id: &str,
) -> Result<bool, LedgerError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM archived_conversations WHERE conversation_id = ?1)",
            [conversation_id],
            |row| row.get(0),
        )
        .map_err(storage_error)
}

#[cfg(test)]
#[path = "agent_chat_archive_tests.rs"]
mod tests;
