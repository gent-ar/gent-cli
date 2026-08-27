//! Public provider-neutral read projections backed by the durable agent-chat hierarchy.

use gent_ports::{AgentChatReadLedger, LedgerError};
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationId, AgentChatConversationSummary,
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRun, AgentChatRunState,
    AgentChatSelection, NormalizedTranscriptPage,
};
use rusqlite::OptionalExtension;

use super::{SqliteLedger, queries::storage_error, transcript_ledger};

impl AgentChatReadLedger for SqliteLedger {
    fn read_agent_chat_summary(
        &self,
        conversation_id: &str,
    ) -> Result<AgentChatConversationSummary, LedgerError> {
        summary(self, conversation_id)
    }

    fn read_agent_chat_detail(
        &self,
        conversation_id: &str,
    ) -> Result<AgentChatConversationDetail, LedgerError> {
        detail(self, conversation_id)
    }

    fn read_agent_chat_transcript(
        &self,
        conversation_id: &str,
        after_cursor: Option<u64>,
        limit: u16,
    ) -> Result<NormalizedTranscriptPage, LedgerError> {
        transcript_ledger::page(
            self,
            &AgentChatConversationId(conversation_id.into()),
            after_cursor.unwrap_or_default(),
            limit,
        )
    }
}

fn summary(
    ledger: &SqliteLedger,
    conversation_id: &str,
) -> Result<AgentChatConversationSummary, LedgerError> {
    let connection = ledger.lock()?;
    let selection = connection.query_row("SELECT s.provider, s.model, s.effort, s.mode FROM runs r JOIN agent_chat_run_selections s ON s.run_id = r.run_id WHERE r.conversation_id = ?1 ORDER BY r.rowid DESC LIMIT 1", [conversation_id], selection).optional().map_err(storage_error)?.ok_or_else(|| LedgerError::Invariant("agent chat conversation does not exist".into()))?;
    let title = metadata(&connection, conversation_id, "title")?;
    let recap = metadata(&connection, conversation_id, "recap")?;
    let (workspace_id, workspace_path) = connection
        .query_row(
            "SELECT c.workspace_id, w.canonical_path FROM agent_chat_conversations c LEFT JOIN workspaces w ON w.workspace_id = c.workspace_id WHERE c.conversation_id = ?1",
            [conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(storage_error)?
        .unwrap_or((None, None));
    Ok(AgentChatConversationSummary {
        conversation_id: conversation_id.into(),
        title,
        recap,
        workspace_id,
        workspace_path,
        mcp_server_count: 0,
        mcp_server_names: Vec::new(),
        changed_file_count: None,
        git_branch: None,
        updated_at_unix_ms: 0,
        selection,
    })
}

fn metadata(
    connection: &rusqlite::Connection,
    conversation_id: &str,
    kind: &str,
) -> Result<Option<String>, LedgerError> {
    connection.query_row("SELECT text FROM conversation_artifacts WHERE conversation_id = ?1 AND kind = ?2 AND status = 'completed' AND text IS NOT NULL ORDER BY rowid DESC LIMIT 1", [conversation_id, kind], |row| row.get(0)).optional().map_err(storage_error)
}

fn detail(
    ledger: &SqliteLedger,
    conversation_id: &str,
) -> Result<AgentChatConversationDetail, LedgerError> {
    let summary = summary(ledger, conversation_id)?;
    let connection = ledger.lock()?;
    let mut statement = connection.prepare("SELECT r.run_id, r.parent_run_id, s.provider, s.model, s.effort, s.mode FROM runs r JOIN agent_chat_run_selections s ON s.run_id = r.run_id WHERE r.conversation_id = ?1 ORDER BY r.rowid ASC").map_err(storage_error)?;
    let runs = statement
        .query_map([conversation_id], |row| {
            Ok(AgentChatRun {
                run_id: row.get(0)?,
                parent_run_id: row.get(1)?,
                selection: selection_at(row, 2)?,
                state: AgentChatRunState::Idle,
            })
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    let current_run_id = runs.last().map(|run| run.run_id.clone()).ok_or_else(|| {
        LedgerError::Invariant("agent chat conversation has no selected run".into())
    })?;
    Ok(AgentChatConversationDetail {
        summary,
        current_run_id,
        runs,
    })
}

fn selection(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentChatSelection> {
    selection_at(row, 0)
}

fn selection_at(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<AgentChatSelection> {
    Ok(AgentChatSelection {
        provider: provider(&row.get::<_, String>(start)?).map_err(to_sql_error)?,
        model: row.get(start + 1)?,
        effort: effort(&row.get::<_, String>(start + 2)?).map_err(to_sql_error)?,
        mode: mode(&row.get::<_, String>(start + 3)?).map_err(to_sql_error)?,
    })
}

fn provider(value: &str) -> Result<AgentChatProvider, LedgerError> {
    match value {
        "claude" => Ok(AgentChatProvider::Claude),
        "codex" => Ok(AgentChatProvider::Codex),
        "claurst" => Ok(AgentChatProvider::Claurst),
        _ => Err(LedgerError::Storage("unknown agent chat provider".into())),
    }
}

fn effort(value: &str) -> Result<AgentChatEffort, LedgerError> {
    match value {
        "low" => Ok(AgentChatEffort::Low),
        "medium" => Ok(AgentChatEffort::Medium),
        "high" => Ok(AgentChatEffort::High),
        "xhigh" => Ok(AgentChatEffort::XHigh),
        "max" => Ok(AgentChatEffort::Max),
        "ultra" => Ok(AgentChatEffort::Ultra),
        _ => Err(LedgerError::Storage("unknown agent chat effort".into())),
    }
}

fn mode(value: &str) -> Result<AgentChatMode, LedgerError> {
    match value {
        "ask" => Ok(AgentChatMode::Ask),
        "plan" => Ok(AgentChatMode::Plan),
        "agent" => Ok(AgentChatMode::Agent),
        _ => Err(LedgerError::Storage("unknown agent chat mode".into())),
    }
}

fn to_sql_error(error: LedgerError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}
