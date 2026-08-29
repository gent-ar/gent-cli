//! `SQLite` persistence for immutable per-conversation advanced-launch-configuration revisions.

use gent_ports::LedgerError;
use gent_types::{AgentChatConversationConfigRecord, AgentChatConversationId};
use rusqlite::{OptionalExtension, params};

use super::SqliteLedger;
use super::queries::storage_error;

pub(super) fn save(
    ledger: &SqliteLedger,
    config: &AgentChatConversationConfigRecord,
) -> Result<(), LedgerError> {
    validate(config)?;
    let connection = ledger.lock()?;
    if !conversation_exists(&connection, &config.conversation_id.0)? {
        return Err(LedgerError::Invariant(
            "conversation config conversation is unknown".into(),
        ));
    }
    let latest = current(&connection, &config.conversation_id.0)?;
    let next = latest.as_ref().map_or(Ok(1), |current| {
        current
            .revision
            .checked_add(1)
            .ok_or_else(|| LedgerError::Invariant("conversation config revision overflow".into()))
    })?;
    if config.revision != next {
        return Err(LedgerError::Invariant(format!(
            "conversation config revision must be {next} for this conversation"
        )));
    }
    let disallowed_tools = serde_json::to_string(&config.disallowed_tools)
        .map_err(|error| LedgerError::Storage(error.to_string()))?;
    connection
        .execute(
            "INSERT INTO agent_chat_conversation_configs (conversation_id, revision, system_prompt, append_system_prompt, max_turns, disallowed_tools) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                config.conversation_id.0,
                config.revision,
                config.system_prompt,
                config.append_system_prompt,
                config.max_turns,
                disallowed_tools,
            ],
        )
        .map(|_| ())
        .map_err(storage_error)
}

pub(super) fn current_conversation_config(
    ledger: &SqliteLedger,
    conversation_id: &str,
) -> Result<Option<AgentChatConversationConfigRecord>, LedgerError> {
    let connection = ledger.lock()?;
    current(&connection, conversation_id)
}

fn current(
    connection: &rusqlite::Connection,
    conversation_id: &str,
) -> Result<Option<AgentChatConversationConfigRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT conversation_id, revision, system_prompt, append_system_prompt, max_turns, disallowed_tools FROM agent_chat_conversation_configs WHERE conversation_id = ?1 ORDER BY revision DESC LIMIT 1",
            [conversation_id],
            decode_config,
        )
        .optional()
        .map_err(storage_error)
}

fn conversation_exists(
    connection: &rusqlite::Connection,
    conversation_id: &str,
) -> Result<bool, LedgerError> {
    connection
        .query_row(
            "SELECT 1 FROM agent_chat_conversations WHERE conversation_id = ?1",
            [conversation_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)
        .map(|row| row.is_some())
}

fn validate(config: &AgentChatConversationConfigRecord) -> Result<(), LedgerError> {
    if config.conversation_id.0.is_empty() || config.revision == 0 {
        return Err(LedgerError::Invariant(
            "conversation config identity and non-zero revision are required".into(),
        ));
    }
    if config.disallowed_tools.iter().any(|tool| tool.is_empty())
        || config
            .disallowed_tools
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(LedgerError::Invariant(
            "conversation config disallowed tools must be canonical, sorted, and unique".into(),
        ));
    }
    if config.max_turns == Some(0) {
        return Err(LedgerError::Invariant(
            "conversation config max turns must be positive when set".into(),
        ));
    }
    Ok(())
}

fn decode_config(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentChatConversationConfigRecord> {
    let disallowed_tools = row.get::<_, String>(5)?;
    Ok(AgentChatConversationConfigRecord {
        conversation_id: AgentChatConversationId(row.get(0)?),
        revision: row.get(1)?,
        system_prompt: row.get(2)?,
        append_system_prompt: row.get(3)?,
        max_turns: row.get(4)?,
        disallowed_tools: serde_json::from_str(&disallowed_tools)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}
