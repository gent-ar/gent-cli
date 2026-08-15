//! `SQLite` persistence for MCP connector records and exclusive source leases.

use gent_ports::{LedgerError, McpConnectorLease, McpConnectorLeaseClaim, McpConnectorUpdate};
use gent_types::{HostEpoch, McpConnectorPhase, McpConnectorRecord};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::SqliteLedger;
use super::epoch::require_epoch;
use super::queries::{host_ingress, storage_error};

pub(super) fn create(
    ledger: &SqliteLedger,
    connector: &McpConnectorRecord,
) -> Result<(), LedgerError> {
    if connector.phase != McpConnectorPhase::Requested || !valid(connector) {
        return Err(LedgerError::Invariant(
            "MCP connector requires safe identities and requested phase".into(),
        ));
    }
    let connection = ledger.lock()?;
    let source = connection
        .query_row(
            "SELECT workspace_id, kind FROM tool_sources WHERE tool_source_id = ?1",
            [&connector.tool_source_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(storage_error)?;
    if !matches!(source, Some((workspace, kind)) if workspace == connector.workspace_id && kind == "mcpServer")
    {
        return Err(LedgerError::Invariant(
            "MCP connector source must be an existing MCP declaration in its workspace".into(),
        ));
    }
    connection
        .execute(
            "INSERT INTO mcp_connectors (connector_id, workspace_id, tool_source_id, phase) VALUES (?1, ?2, ?3, ?4)",
            params![connector.connector_id, connector.workspace_id, connector.tool_source_id, encode_phase(connector.phase)],
        )
        .map(|_| ())
        .map_err(storage_error)
}

pub(super) fn find(
    ledger: &SqliteLedger,
    connector_id: &str,
) -> Result<Option<McpConnectorRecord>, LedgerError> {
    let connection = ledger.lock()?;
    find_connection(&connection, connector_id)
}

pub(super) fn replace_phase(
    ledger: &SqliteLedger,
    connector_id: &str,
    expected: McpConnectorPhase,
    next: McpConnectorPhase,
) -> Result<McpConnectorUpdate, LedgerError> {
    let connection = ledger.lock()?;
    let current = find_connection(&connection, connector_id)?
        .ok_or_else(|| LedgerError::Invariant("MCP connector does not exist".into()))?;
    if current.phase != expected {
        return Ok(McpConnectorUpdate::Current(current));
    }
    connection
        .execute(
            "UPDATE mcp_connectors SET phase = ?1 WHERE connector_id = ?2 AND phase = ?3",
            params![encode_phase(next), connector_id, encode_phase(expected)],
        )
        .map_err(storage_error)?;
    Ok(McpConnectorUpdate::Applied(McpConnectorRecord {
        phase: next,
        ..current
    }))
}

pub(super) fn claim_lease(
    ledger: &SqliteLedger,
    requested: &McpConnectorLease,
) -> Result<McpConnectorLeaseClaim, LedgerError> {
    if !valid_identity(&requested.tool_source_id) || !valid_identity(&requested.lease_token) {
        return Err(LedgerError::Invariant(
            "MCP lease identities are invalid".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let active = host_ingress(&transaction)?.epoch;
    require_epoch(requested.host_epoch, active)?;
    let result = match find_lease_connection(&transaction, &requested.tool_source_id)? {
        None => {
            transaction.execute(
                "INSERT INTO mcp_connector_leases (tool_source_id, lease_token, host_epoch) VALUES (?1, ?2, ?3)",
                params![requested.tool_source_id, requested.lease_token, requested.host_epoch.0],
            ).map_err(storage_error)?;
            McpConnectorLeaseClaim::Acquired(requested.clone())
        }
        Some(existing) if existing.host_epoch == active => {
            McpConnectorLeaseClaim::Contended(existing)
        }
        Some(previous) => {
            transaction.execute(
                "UPDATE mcp_connector_leases SET lease_token = ?1, host_epoch = ?2 WHERE tool_source_id = ?3",
                params![requested.lease_token, requested.host_epoch.0, requested.tool_source_id],
            ).map_err(storage_error)?;
            McpConnectorLeaseClaim::Recovered {
                previous,
                current: requested.clone(),
            }
        }
    };
    transaction.commit().map_err(storage_error)?;
    Ok(result)
}

pub(super) fn find_lease(
    ledger: &SqliteLedger,
    tool_source_id: &str,
) -> Result<Option<McpConnectorLease>, LedgerError> {
    let connection = ledger.lock()?;
    find_lease_connection(&connection, tool_source_id)
}

fn find_connection(
    connection: &rusqlite::Connection,
    connector_id: &str,
) -> Result<Option<McpConnectorRecord>, LedgerError> {
    connection.query_row(
        "SELECT connector_id, workspace_id, tool_source_id, phase FROM mcp_connectors WHERE connector_id = ?1",
        [connector_id], decode,
    ).optional().map_err(storage_error)
}

fn find_lease_connection(
    connection: &rusqlite::Connection,
    tool_source_id: &str,
) -> Result<Option<McpConnectorLease>, LedgerError> {
    connection.query_row(
        "SELECT tool_source_id, lease_token, host_epoch FROM mcp_connector_leases WHERE tool_source_id = ?1",
        [tool_source_id],
        |row| Ok(McpConnectorLease { tool_source_id: row.get(0)?, lease_token: row.get(1)?, host_epoch: HostEpoch(row.get(2)?) }),
    ).optional().map_err(storage_error)
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<McpConnectorRecord> {
    Ok(McpConnectorRecord {
        connector_id: row.get(0)?,
        workspace_id: row.get(1)?,
        tool_source_id: row.get(2)?,
        phase: decode_phase(&row.get::<_, String>(3)?)?,
    })
}

fn valid(connector: &McpConnectorRecord) -> bool {
    [
        &connector.connector_id,
        &connector.workspace_id,
        &connector.tool_source_id,
    ]
    .into_iter()
    .all(|identity| valid_identity(identity))
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
}

const fn encode_phase(phase: McpConnectorPhase) -> &'static str {
    match phase {
        McpConnectorPhase::Requested => "requested",
        McpConnectorPhase::Connecting => "connecting",
        McpConnectorPhase::Ready => "ready",
        McpConnectorPhase::Failed => "failed",
        McpConnectorPhase::Interrupted => "interrupted",
    }
}

fn decode_phase(phase: &str) -> rusqlite::Result<McpConnectorPhase> {
    match phase {
        "requested" => Ok(McpConnectorPhase::Requested),
        "connecting" => Ok(McpConnectorPhase::Connecting),
        "ready" => Ok(McpConnectorPhase::Ready),
        "failed" => Ok(McpConnectorPhase::Failed),
        "interrupted" => Ok(McpConnectorPhase::Interrupted),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
