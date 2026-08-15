//! `SQLite` persistence for immutable, credential-free tool-source declarations.

use gent_ports::LedgerError;
use gent_types::{ToolSourceKind, ToolSourceRecord};
use rusqlite::params;

use super::SqliteLedger;
use super::queries::storage_error;

pub(super) fn create(ledger: &SqliteLedger, source: &ToolSourceRecord) -> Result<(), LedgerError> {
    validate(source)?;
    let tools = serde_json::to_string(&source.declared_tools)
        .map_err(|error| LedgerError::Storage(error.to_string()))?;
    ledger
        .lock()?
        .execute(
            "INSERT INTO tool_sources (tool_source_id, workspace_id, kind, source_name, declared_tools) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                source.tool_source_id,
                source.workspace_id,
                encode_kind(source.kind),
                source.source_name,
                tools
            ],
        )
        .map(|_| ())
        .map_err(storage_error)
}

pub(super) fn list(
    ledger: &SqliteLedger,
    workspace_id: &str,
) -> Result<Vec<ToolSourceRecord>, LedgerError> {
    let connection = ledger.lock()?;
    let mut statement = connection
        .prepare(
            "SELECT tool_source_id, workspace_id, kind, source_name, declared_tools FROM tool_sources WHERE workspace_id = ?1 ORDER BY rowid",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map([workspace_id], decode_source)
        .map_err(storage_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
}

fn validate(source: &ToolSourceRecord) -> Result<(), LedgerError> {
    if source.tool_source_id.is_empty()
        || source.workspace_id.is_empty()
        || !valid_name(&source.source_name)
        || source.declared_tools.is_empty()
        || source
            .declared_tools
            .iter()
            .any(|tool| !valid_tool_name(tool))
        || source
            .declared_tools
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(LedgerError::Invariant(
            "tool source requires identities, a canonical name, and sorted unique tool names"
                .into(),
        ));
    }
    Ok(())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_tool_name(tool: &str) -> bool {
    !tool.is_empty()
        && tool.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
}

fn encode_kind(kind: ToolSourceKind) -> &'static str {
    match kind {
        ToolSourceKind::McpServer => "mcpServer",
        ToolSourceKind::BuiltIn => "builtIn",
        ToolSourceKind::HostIntegration => "hostIntegration",
    }
}

fn decode_source(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolSourceRecord> {
    let tools = row.get::<_, String>(4)?;
    Ok(ToolSourceRecord {
        tool_source_id: row.get(0)?,
        workspace_id: row.get(1)?,
        kind: decode_kind(&row.get::<_, String>(2)?)?,
        source_name: row.get(3)?,
        declared_tools: serde_json::from_str(&tools).map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}

fn decode_kind(kind: &str) -> rusqlite::Result<ToolSourceKind> {
    match kind {
        "mcpServer" => Ok(ToolSourceKind::McpServer),
        "builtIn" => Ok(ToolSourceKind::BuiltIn),
        "hostIntegration" => Ok(ToolSourceKind::HostIntegration),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
