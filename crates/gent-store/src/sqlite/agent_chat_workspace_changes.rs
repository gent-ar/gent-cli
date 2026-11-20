use gent_ports::LedgerError;
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, Event, HostEpoch, ReceiptId,
    WorkspaceFileChange,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{
    SqliteLedger,
    queries::{append_event, find_event, storage_error},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceChangeTurn {
    pub projection_cursor: u64,
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub host_epoch: HostEpoch,
    pub workspace_path: String,
    pub baseline_tree: Option<String>,
}

fn baseline_event_id(conversation_id: &str) -> String {
    format!("agent-chat-workspace-baseline:{conversation_id}")
}

impl SqliteLedger {
    pub fn agent_chat_projection_head(&self) -> Result<u64, LedgerError> {
        self.lock()?
            .query_row(
                "SELECT COALESCE(MAX(cursor), 0) FROM agent_chat_projection_events",
                [],
                |row| row.get(0),
            )
            .map_err(storage_error)
    }

    pub fn conversation_workspace_path(
        &self,
        conversation_id: &str,
    ) -> Result<Option<String>, LedgerError> {
        self.lock()?
            .query_row(
                "SELECT w.canonical_path FROM agent_chat_conversations c JOIN workspaces w ON w.workspace_id = c.workspace_id WHERE c.conversation_id = ?1",
                [conversation_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage_error)
    }

    pub fn has_workspace_baseline(&self, conversation_id: &str) -> Result<bool, LedgerError> {
        Ok(find_event(&*self.lock()?, &baseline_event_id(conversation_id))?.is_some())
    }

    pub fn record_workspace_baseline(
        &self,
        conversation_id: &str,
        tree: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let event_id = baseline_event_id(conversation_id);
        if find_event(&transaction, &event_id)?.is_some() {
            return Ok(());
        }
        append_event(
            &transaction,
            &Event {
                cursor: 0,
                receipt_id: ReceiptId(event_id.clone()),
                event_id,
                host_epoch,
                kind: "agentChatWorkspaceBaseline".into(),
                payload: serde_json::json!({ "conversationId": conversation_id, "tree": tree }),
            },
        )?;
        transaction.commit().map_err(storage_error)
    }

    pub fn workspace_change_turns(
        &self,
        after_cursor: u64,
        limit: usize,
    ) -> Result<Vec<WorkspaceChangeTurn>, LedgerError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT p.cursor, p.conversation_id, json_extract(p.payload, '$.activity.runId'), json_extract(p.payload, '$.activity.turnId'), json_extract(p.payload, '$.activity.hostEpoch'), w.canonical_path, json_extract(b.payload, '$.tree') FROM agent_chat_projection_events p JOIN agent_chat_conversations c ON c.conversation_id = p.conversation_id JOIN workspaces w ON w.workspace_id = c.workspace_id LEFT JOIN events b ON b.event_id = 'agent-chat-workspace-baseline:' || p.conversation_id WHERE p.kind = 'activity' AND json_extract(p.payload, '$.activity.type') = 'terminal' AND p.cursor > ?1 ORDER BY p.cursor LIMIT ?2")
            .map_err(storage_error)?;
        statement
            .query_map(
                params![after_cursor, i64::try_from(limit).unwrap_or(i64::MAX)],
                |row| {
                    Ok(WorkspaceChangeTurn {
                        projection_cursor: row.get(0)?,
                        conversation_id: row.get(1)?,
                        run_id: row.get(2)?,
                        turn_id: row.get(3)?,
                        host_epoch: HostEpoch(row.get(4)?),
                        workspace_path: row.get(5)?,
                        baseline_tree: row.get(6)?,
                    })
                },
            )
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)
    }

    pub fn record_workspace_changes(
        &self,
        turn: &WorkspaceChangeTurn,
        files: Vec<WorkspaceFileChange>,
        truncated: bool,
    ) -> Result<(), LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let event_id = format!("agent-chat-workspace-changes:{}", turn.turn_id);
        if find_event(&transaction, &event_id)?.is_some() {
            return Ok(());
        }
        super::agent_chat_queue_activity::append_activity(
            &transaction,
            event_id.clone(),
            ReceiptId(event_id),
            "agentChatWorkspaceActivity",
            ConversationActivityFact::WorkspaceChanges {
                scope: ConversationActivityScope {
                    conversation_id: turn.conversation_id.clone(),
                    run_id: turn.run_id.clone(),
                    turn_id: turn.turn_id.clone(),
                    host_epoch: turn.host_epoch,
                    cursor: 0,
                },
                files,
                truncated,
            },
        )?;
        transaction.commit().map_err(storage_error)
    }
}

#[cfg(test)]
#[path = "agent_chat_workspace_changes_tests.rs"]
mod tests;
