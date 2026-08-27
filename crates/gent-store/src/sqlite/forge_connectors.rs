use gent_ports::LedgerError;
use gent_types::{ForgeConnectorRecord, McpConnectorPhase};
use rusqlite::{OptionalExtension, params};

use super::{SqliteLedger, queries::storage_error};

pub(super) fn create(
    ledger: &SqliteLedger,
    connector: &ForgeConnectorRecord,
) -> Result<(), LedgerError> {
    validate(connector)?;
    let connection = ledger.lock()?;
    let source = connection
        .query_row(
            "SELECT workspace_id, tool_source_id, phase FROM mcp_connectors WHERE connector_id = ?1",
            [&connector.connector_id],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            )),
        )
        .optional()
        .map_err(storage_error)?;
    if !matches!(source, Some((workspace, tool_source, phase)) if workspace == connector.workspace_id && tool_source == connector.tool_source_id && phase == phase_name(connector.phase))
    {
        return Err(LedgerError::Invariant(
            "Forge connector requires a matching MCP connector".into(),
        ));
    }
    connection
        .execute(
            "INSERT INTO forge_connectors (connector_id, workspace_id, tool_source_id, name, description, category, phase, declared_tools, discovered_tools, enabled) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![connector.connector_id, connector.workspace_id, connector.tool_source_id, connector.name, connector.description, connector.category, phase_name(connector.phase), tools(&connector.declared_tools)?, tools(&connector.discovered_tools)?, connector.enabled],
        )
        .map(|_| ())
        .map_err(storage_error)
}

pub(super) fn find(
    ledger: &SqliteLedger,
    connector_id: &str,
) -> Result<Option<ForgeConnectorRecord>, LedgerError> {
    ledger
        .lock()?
        .query_row(
            "SELECT connector_id, workspace_id, tool_source_id, name, description, category, phase, declared_tools, discovered_tools, enabled FROM forge_connectors WHERE connector_id = ?1",
            [connector_id],
            decode,
        )
        .optional()
        .map_err(storage_error)
}

pub(super) fn list(
    ledger: &SqliteLedger,
    workspace_id: &str,
) -> Result<Vec<ForgeConnectorRecord>, LedgerError> {
    let connection = ledger.lock()?;
    let mut statement = connection
        .prepare("SELECT connector_id, workspace_id, tool_source_id, name, description, category, phase, declared_tools, discovered_tools, enabled FROM forge_connectors WHERE workspace_id = ?1 ORDER BY rowid")
        .map_err(storage_error)?;
    let rows = statement
        .query_map([workspace_id], decode)
        .map_err(storage_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
}

pub(super) fn replace(
    ledger: &SqliteLedger,
    connector: &ForgeConnectorRecord,
) -> Result<(), LedgerError> {
    validate(connector)?;
    let changed = ledger
        .lock()?
        .execute(
            "UPDATE forge_connectors SET workspace_id = ?1, tool_source_id = ?2, name = ?3, description = ?4, category = ?5, phase = ?6, declared_tools = ?7, discovered_tools = ?8, enabled = ?9 WHERE connector_id = ?10",
            params![connector.workspace_id, connector.tool_source_id, connector.name, connector.description, connector.category, phase_name(connector.phase), tools(&connector.declared_tools)?, tools(&connector.discovered_tools)?, connector.enabled, connector.connector_id],
        )
        .map_err(storage_error)?;
    (changed == 1)
        .then_some(())
        .ok_or_else(|| LedgerError::Invariant("Forge connector does not exist".into()))
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<ForgeConnectorRecord> {
    Ok(ForgeConnectorRecord {
        connector_id: row.get(0)?,
        workspace_id: row.get(1)?,
        tool_source_id: row.get(2)?,
        name: row.get(3)?,
        description: row.get(4)?,
        category: row.get(5)?,
        phase: decode_phase(&row.get::<_, String>(6)?)?,
        declared_tools: serde_json::from_str(&row.get::<_, String>(7)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        discovered_tools: serde_json::from_str(&row.get::<_, String>(8)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        enabled: row.get(9)?,
    })
}

fn validate(connector: &ForgeConnectorRecord) -> Result<(), LedgerError> {
    if !valid_identity(&connector.connector_id)
        || !valid_identity(&connector.workspace_id)
        || !valid_identity(&connector.tool_source_id)
        || !valid_name(&connector.name)
        || !valid_name(&connector.category)
        || !valid_tools(&connector.declared_tools)
        || !valid_tools(&connector.discovered_tools)
        || !connector
            .discovered_tools
            .iter()
            .all(|tool| connector.declared_tools.binary_search(tool).is_ok())
    {
        return Err(LedgerError::Invariant(
            "Forge connector metadata is invalid".into(),
        ));
    }
    Ok(())
}

fn valid_tools(tools: &[String]) -> bool {
    tools.windows(2).all(|pair| pair[0] < pair[1]) && tools.iter().all(|tool| valid_identity(tool))
}

fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 160 && !value.chars().any(char::is_control)
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
}

fn tools(value: &[String]) -> Result<String, LedgerError> {
    serde_json::to_string(value).map_err(|error| LedgerError::Storage(error.to_string()))
}

const fn phase_name(phase: McpConnectorPhase) -> &'static str {
    match phase {
        McpConnectorPhase::Requested => "requested",
        McpConnectorPhase::Connecting => "connecting",
        McpConnectorPhase::Ready => "ready",
        McpConnectorPhase::Failed => "failed",
        McpConnectorPhase::Interrupted => "interrupted",
    }
}

fn decode_phase(value: &str) -> rusqlite::Result<McpConnectorPhase> {
    match value {
        "requested" => Ok(McpConnectorPhase::Requested),
        "connecting" => Ok(McpConnectorPhase::Connecting),
        "ready" => Ok(McpConnectorPhase::Ready),
        "failed" => Ok(McpConnectorPhase::Failed),
        "interrupted" => Ok(McpConnectorPhase::Interrupted),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

#[cfg(test)]
mod tests {
    use gent_ports::{ForgeConnectorLedger, McpConnectorLedger, ToolSourceLedger, WorkspaceLedger};
    use gent_types::{
        ForgeConnectorRecord, McpConnectorPhase, McpConnectorRecord, ToolSourceKind,
        ToolSourceRecord, WorkspaceRecord,
    };

    use crate::SqliteLedger;

    fn connector() -> ForgeConnectorRecord {
        ForgeConnectorRecord {
            connector_id: "connector-1".into(),
            workspace_id: "workspace-1".into(),
            tool_source_id: "source-1".into(),
            name: "gent-forge".into(),
            description: "Gent Forge".into(),
            category: "gent".into(),
            phase: McpConnectorPhase::Requested,
            declared_tools: vec!["gent-forge.apply".into(), "gent-forge.plan".into()],
            discovered_tools: vec!["gent-forge.plan".into()],
            enabled: true,
        }
    }

    #[test]
    fn persists_only_a_matching_mcp_connector() {
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_workspace(&WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/tmp/project".into(),
            })
            .unwrap();
        ledger
            .create_tool_source(&ToolSourceRecord {
                tool_source_id: "source-1".into(),
                workspace_id: "workspace-1".into(),
                kind: ToolSourceKind::McpServer,
                source_name: "gent-forge".into(),
                declared_tools: vec!["gent-forge.apply".into(), "gent-forge.plan".into()],
            })
            .unwrap();
        ledger
            .create_mcp_connector(&McpConnectorRecord {
                connector_id: "connector-1".into(),
                workspace_id: "workspace-1".into(),
                tool_source_id: "source-1".into(),
                phase: McpConnectorPhase::Requested,
            })
            .unwrap();
        let value = connector();
        ledger.create_forge_connector(&value).unwrap();
        assert_eq!(
            ledger.list_forge_connectors("workspace-1").unwrap(),
            vec![value]
        );
        let mut invalid = connector();
        invalid.discovered_tools = vec!["gent-forge.unknown".into()];
        assert!(ledger.replace_forge_connector(&invalid).is_err());
    }
}
