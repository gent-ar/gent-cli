use gent_ports::{IngressMode, LedgerError};
use gent_types::{
    Command, Event, ForgeConnectorRecord, McpConnectorPhase, McpConnectorRecord, Receipt,
    ReceiptStatus, ToolSourceKind, ToolSourceRecord,
};
use rusqlite::{TransactionBehavior, params};

use super::super::{
    SqliteLedger, epoch::require_epoch, forge_connectors, mcp_connectors, provision_receipts,
    queries, tool_sources,
};

pub(super) fn commit(
    ledger: &SqliteLedger,
    command: &Command,
    source: &ToolSourceRecord,
    connector: &ForgeConnectorRecord,
    conversation_id: &str,
    run_id: &str,
) -> Result<Receipt, LedgerError> {
    if command.kind != "managedMcpRegister"
        || source.kind != ToolSourceKind::McpServer
        || source.tool_source_id != connector.tool_source_id
        || source.workspace_id != connector.workspace_id
        || source.declared_tools != connector.declared_tools
        || connector.phase != McpConnectorPhase::Requested
        || connector.enabled
        || !connector.discovered_tools.is_empty()
        || command.payload["conversationId"] != conversation_id
        || command.payload["runId"] != run_id
        || command.payload["workspaceId"] != source.workspace_id
        || command.payload["connectorId"] != connector.connector_id
        || command.payload["toolSourceId"] != source.tool_source_id
    {
        return Err(invalid());
    }
    let object = command.payload.as_object().ok_or_else(invalid)?;
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    if keys
        != [
            "artifactDigest",
            "connectorId",
            "conversationId",
            "registrationDigest",
            "runId",
            "toolSourceId",
            "workspaceId",
        ]
    {
        return Err(invalid());
    }
    for field in ["artifactDigest", "registrationDigest"] {
        if !command.payload[field].as_str().is_some_and(|value| {
            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(invalid());
        }
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(queries::storage_error)?;
    let ingress = queries::host_ingress(&transaction)?;
    require_epoch(command.host_epoch, ingress.epoch)?;
    if ingress.mode != IngressMode::Open {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    let mut receipt =
        provision_receipts::bound_receipt(&transaction, command)?.ok_or_else(invalid)?;
    if receipt.status != ReceiptStatus::Accepted {
        return Err(invalid());
    }
    let owner: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_chat_conversations c JOIN runs r ON r.conversation_id=c.conversation_id WHERE c.conversation_id=?1 AND c.workspace_id=?2 AND r.run_id=?3 AND r.rowid=(SELECT MAX(current.rowid) FROM runs current WHERE current.conversation_id=c.conversation_id))",
        params![conversation_id, source.workspace_id, run_id], |row| row.get(0),
    ).map_err(queries::storage_error)?;
    if !owner {
        return Err(invalid());
    }
    tool_sources::insert(&transaction, source)?;
    mcp_connectors::insert(
        &transaction,
        &McpConnectorRecord {
            connector_id: connector.connector_id.clone(),
            workspace_id: connector.workspace_id.clone(),
            tool_source_id: connector.tool_source_id.clone(),
            phase: McpConnectorPhase::Requested,
        },
    )?;
    forge_connectors::insert(&transaction, connector)?;
    transaction
        .execute(
            "UPDATE receipts SET status='settled' WHERE idempotency_key=?1 AND status='accepted'",
            [&command.idempotency_key],
        )
        .map_err(queries::storage_error)?;
    queries::append_event(
        &transaction,
        &Event {
            cursor: 0,
            event_id: format!("managed-mcp-registration-prepared:{}", command.receipt_id.0),
            receipt_id: command.receipt_id.clone(),
            host_epoch: command.host_epoch,
            kind: "managedMcpRegistrationPrepared".into(),
            payload: serde_json::json!({
                "connectorId": connector.connector_id, "workspaceId": connector.workspace_id,
                "toolSourceId": connector.tool_source_id,
            }),
        },
    )?;
    transaction.commit().map_err(queries::storage_error)?;
    receipt.status = ReceiptStatus::Settled;
    Ok(receipt)
}

fn invalid() -> LedgerError {
    LedgerError::Invariant("managed MCP registration binding is invalid".into())
}

#[cfg(test)]
#[path = "managed_forge_registration_tests.rs"]
mod tests;
