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
    connection.query_row("SELECT provider, model, effort, mode FROM agent_chat_conversations WHERE conversation_id = ?1", [conversation_id], selection).optional().map_err(storage_error)?.map(|selection| AgentChatConversationSummary { conversation_id: conversation_id.into(), title: None, updated_at_unix_ms: 0, selection }).ok_or_else(|| LedgerError::Invariant("agent chat conversation does not exist".into()))
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
