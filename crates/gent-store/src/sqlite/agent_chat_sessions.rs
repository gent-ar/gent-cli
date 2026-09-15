use gent_ports::{AgentChatSessionLedger, LedgerError};
use gent_types::{AgentChatSession, AgentChatSessionId};
use rusqlite::{Connection, OptionalExtension, params};

use super::{SqliteLedger, queries::storage_error};

impl AgentChatSessionLedger for SqliteLedger {
    fn create_agent_chat_session(&self, session: &AgentChatSession) -> Result<(), LedgerError> {
        session
            .validate()
            .map_err(|error| LedgerError::Invariant(error.into()))?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        transaction
            .execute(
                "INSERT INTO agent_chat_sessions (session_id, workspace_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session.session_id.0, session.workspace_id, session.name, session.created_at, session.updated_at],
            )
            .map_err(storage_error)?;
        for (ordinal, conversation_id) in session.conversation_ids.iter().enumerate() {
            let ordinal = i64::try_from(ordinal).map_err(|_| {
                LedgerError::Invariant(
                    "session conversation count exceeds SQLite ordinal range".into(),
                )
            })?;
            transaction
                .execute(
                    "INSERT INTO agent_chat_session_conversations (session_id, conversation_id, ordinal) VALUES (?1, ?2, ?3)",
                    params![session.session_id.0, conversation_id, ordinal],
                )
                .map_err(storage_error)?;
        }
        transaction.commit().map_err(storage_error)
    }

    fn find_agent_chat_session(
        &self,
        session_id: &AgentChatSessionId,
    ) -> Result<Option<AgentChatSession>, LedgerError> {
        let connection = self.lock()?;
        read_session(&connection, &session_id.0)
    }

    fn list_agent_chat_sessions(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<AgentChatSession>, LedgerError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare("SELECT session_id FROM agent_chat_sessions WHERE workspace_id = ?1 ORDER BY updated_at DESC").map_err(storage_error)?;
        let ids = statement
            .query_map([workspace_id], |row| row.get::<_, String>(0))
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        ids.iter()
            .map(|id| {
                read_session(&connection, id)?
                    .ok_or_else(|| LedgerError::Invariant("session disappeared".into()))
            })
            .collect()
    }

    fn attach_agent_chat_conversation(
        &self,
        session_id: &AgentChatSessionId,
        conversation_id: &str,
    ) -> Result<AgentChatSession, LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let ordinal: i64 = transaction.query_row("SELECT COALESCE(MAX(ordinal), -1) + 1 FROM agent_chat_session_conversations WHERE session_id = ?1", [&session_id.0], |row| row.get(0)).map_err(storage_error)?;
        transaction.execute("INSERT INTO agent_chat_session_conversations (session_id, conversation_id, ordinal) VALUES (?1, ?2, ?3)", params![session_id.0, conversation_id, ordinal]).map_err(storage_error)?;
        transaction
            .execute(
                "UPDATE agent_chat_sessions SET updated_at = updated_at + 1 WHERE session_id = ?1",
                [&session_id.0],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        read_session(&connection, &session_id.0)?
            .ok_or_else(|| LedgerError::Invariant("session does not exist".into()))
    }
}

fn read_session(
    connection: &Connection,
    session_id: &str,
) -> Result<Option<AgentChatSession>, LedgerError> {
    let mut session = connection.query_row(
        "SELECT session_id, workspace_id, name, created_at, updated_at FROM agent_chat_sessions WHERE session_id = ?1",
        [session_id],
        |row| Ok(AgentChatSession {
            session_id: AgentChatSessionId(row.get(0)?),
            workspace_id: row.get(1)?,
            name: row.get(2)?,
            conversation_ids: Vec::new(),
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        }),
    ).optional().map_err(storage_error)?;
    if let Some(value) = &mut session {
        let mut statement = connection.prepare("SELECT conversation_id FROM agent_chat_session_conversations WHERE session_id = ?1 ORDER BY ordinal").map_err(storage_error)?;
        value.conversation_ids = statement
            .query_map([session_id], |row| row.get(0))
            .map_err(storage_error)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(storage_error)?;
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use gent_ports::{AgentChatSessionLedger, AgentChatWorkspaceLedger};
    use gent_types::{
        AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
        AgentChatProvider, AgentChatRunId, AgentChatSelection, AgentChatSession,
        AgentChatSessionId, HostEpoch, ReceiptId, WorkspaceRecord,
    };

    use super::SqliteLedger;

    fn ledger_with_conversation() -> SqliteLedger {
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("receipt".into()),
                    idempotency_key: "key".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: AgentChatConversationId("conversation".into()),
                    run_id: AgentChatRunId("run".into()),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Claurst,
                        model: "qwen3-1-7b-q4-k-m".into(),
                        effort: AgentChatEffort::Medium,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace".into(),
                    canonical_path: "/workspace".into(),
                },
            )
            .unwrap();
        ledger
    }

    #[test]
    fn attaching_a_conversation_returns_the_updated_session() {
        let ledger = ledger_with_conversation();
        ledger
            .create_agent_chat_session(&AgentChatSession {
                session_id: AgentChatSessionId("session".into()),
                workspace_id: "workspace".into(),
                name: "Session".into(),
                conversation_ids: Vec::new(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        let session = ledger
            .attach_agent_chat_conversation(&AgentChatSessionId("session".into()), "conversation")
            .unwrap();
        assert_eq!(session.conversation_ids, ["conversation"]);
    }

    #[test]
    fn listing_sessions_returns_while_a_session_exists() {
        let ledger = ledger_with_conversation();
        ledger
            .create_agent_chat_session(&AgentChatSession {
                session_id: AgentChatSessionId("session".into()),
                workspace_id: "workspace".into(),
                name: "Session".into(),
                conversation_ids: Vec::new(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ledger.list_agent_chat_sessions("workspace"));
        });
        let sessions = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("listing sessions must not block on the ledger connection")
            .unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "Session");
    }

    #[test]
    fn creating_a_session_retains_its_initial_conversation() {
        let ledger = ledger_with_conversation();
        ledger
            .create_agent_chat_session(&AgentChatSession {
                session_id: AgentChatSessionId("session".into()),
                workspace_id: "workspace".into(),
                name: "Session".into(),
                conversation_ids: vec!["conversation".into()],
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        let session = ledger
            .find_agent_chat_session(&AgentChatSessionId("session".into()))
            .unwrap()
            .unwrap();
        assert_eq!(session.conversation_ids, ["conversation"]);
    }
}
