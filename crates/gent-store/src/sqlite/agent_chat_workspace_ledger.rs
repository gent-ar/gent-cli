//! Workspace-bound creation and resolution for durable agent-chat conversations.

use gent_ports::{AgentChatWorkspaceLedger, LedgerError};
use gent_types::{AgentChatConversationCreate, AgentChatConversationCreated, WorkspaceRecord};
use rusqlite::params;

use super::SqliteLedger;
use super::agent_chat_ledger::create_conversation;
use super::queries::storage_error;

impl AgentChatWorkspaceLedger for SqliteLedger {
    fn create_agent_chat_conversation_in_workspace(
        &self,
        create: &AgentChatConversationCreate,
        workspace: &WorkspaceRecord,
    ) -> Result<AgentChatConversationCreated, LedgerError> {
        create_conversation(self, create, Some(workspace))
    }

    fn agent_chat_workspace_for_run(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<WorkspaceRecord, LedgerError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT w.workspace_id, w.canonical_path FROM agent_chat_conversations c JOIN runs r ON r.conversation_id = c.conversation_id JOIN workspaces w ON w.workspace_id = c.workspace_id WHERE c.conversation_id = ?1 AND r.run_id = ?2",
                params![conversation_id, run_id],
                |row| {
                    Ok(WorkspaceRecord {
                        workspace_id: row.get(0)?,
                        canonical_path: row.get(1)?,
                    })
                },
            )
            .map_err(storage_error)
    }
}
