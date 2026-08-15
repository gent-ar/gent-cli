//! Receipt-backed, lease-fenced orchestration for one credential-free MCP connection.
use std::sync::{Arc, Mutex};

use gent_ports::{
    Ledger, McpConnectOperation, McpConnectionSummary, McpConnectorExecutor, McpConnectorLease,
    McpConnectorLeaseClaim, McpConnectorLedger, McpConnectorUpdate, ReceiptClaim, ToolSourceLedger,
};
use gent_types::{
    Command, Event, HostEpoch, McpConnectorPhase, McpConnectorRecord, Receipt, ReceiptId,
    ReceiptStatus, ToolSourceKind,
};

use crate::RuntimeError;

/// Explicit receipt, connector, and lease identity for one connection request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpConnectRequest {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    pub connector: McpConnectorRecord,
    pub lease: McpConnectorLease,
}

/// Terminal state reported by the narrow MCP connection service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpConnectState {
    DeniedObserver,
    Connected,
    Failed,
    Rejected,
    Unprovable,
}

/// Receipt result. It intentionally exposes only the negotiated tool count.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpConnectResult {
    pub state: McpConnectState,
    pub receipt: Option<Receipt>,
    pub summary: Option<McpConnectionSummary>,
}

/// Serializes a single source connection after durable receipt and connector-lease ownership.
#[derive(Clone, Debug)]
pub struct McpConnectorService<L, E> {
    ledger: L,
    executor: E,
    authority: bool,
    serial: Arc<Mutex<()>>,
}

impl<L, E> McpConnectorService<L, E> {
    /// Creates a service. `authority = false` is a no-write, no-connect observer denial.
    #[must_use]
    pub fn new(ledger: L, executor: E, authority: bool) -> Self {
        Self {
            ledger,
            executor,
            authority,
            serial: Arc::new(Mutex::new(())),
        }
    }
}

impl<L: Ledger + McpConnectorLedger + ToolSourceLedger, E: McpConnectorExecutor>
    McpConnectorService<L, E>
{
    /// Claims a receipt and source lease before invoking a future daemon-owned connector.
    ///
    /// An accepted receipt recovered after a restart is terminally `Unprovable`, never replayed.
    ///
    /// # Errors
    /// Returns an error when the durable receipt, source, or lease cannot be read or written.
    pub fn connect(&self, request: &McpConnectRequest) -> Result<McpConnectResult, RuntimeError> {
        if !self.authority {
            return Ok(McpConnectResult {
                state: McpConnectState::DeniedObserver,
                receipt: None,
                summary: None,
            });
        }
        let _serial = self
            .serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let command = command_for(request);
        match self
            .ledger
            .claim_command(&command, &accepted_event(&command))?
        {
            ReceiptClaim::Accepted(receipt) => self.connect_claimed(request, &receipt),
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => {
                self.settle(&receipt, McpConnectState::Unprovable, None)
            }
            ReceiptClaim::Existing(receipt) => self.existing(receipt),
        }
    }

    fn connect_claimed(
        &self,
        request: &McpConnectRequest,
        receipt: &Receipt,
    ) -> Result<McpConnectResult, RuntimeError> {
        if !valid_request(request) {
            return self.settle(receipt, McpConnectState::Rejected, None);
        }
        let Some(source) = self
            .ledger
            .find_tool_source(&request.connector.tool_source_id)?
        else {
            return self.settle(receipt, McpConnectState::Rejected, None);
        };
        if source.workspace_id != request.connector.workspace_id
            || source.kind != ToolSourceKind::McpServer
        {
            return self.settle(receipt, McpConnectState::Rejected, None);
        }
        if matches!(
            self.ledger.claim_mcp_connector_lease(&request.lease)?,
            McpConnectorLeaseClaim::Contended(_)
        ) {
            return self.settle(receipt, McpConnectState::Rejected, None);
        }
        if self
            .ledger
            .create_mcp_connector(&request.connector)
            .is_err()
        {
            return self.settle(receipt, McpConnectState::Rejected, None);
        }
        if !self.transition(
            &request.connector.connector_id,
            McpConnectorPhase::Requested,
            McpConnectorPhase::Connecting,
        )? {
            return self.settle(receipt, McpConnectState::Unprovable, None);
        }
        let outcome = self.executor.connect(&McpConnectOperation {
            tool_source_id: source.tool_source_id,
            source_name: source.source_name,
            declared_tools: source.declared_tools,
        });
        if let Ok(summary) = outcome {
            if !self.transition(
                &request.connector.connector_id,
                McpConnectorPhase::Connecting,
                McpConnectorPhase::Ready,
            )? {
                return self.settle(receipt, McpConnectState::Unprovable, None);
            }
            self.settle(receipt, McpConnectState::Connected, Some(summary))
        } else {
            if !self.transition(
                &request.connector.connector_id,
                McpConnectorPhase::Connecting,
                McpConnectorPhase::Failed,
            )? {
                return self.settle(receipt, McpConnectState::Unprovable, None);
            }
            self.settle(receipt, McpConnectState::Failed, None)
        }
    }

    fn existing(&self, receipt: Receipt) -> Result<McpConnectResult, RuntimeError> {
        let state = match receipt.status {
            ReceiptStatus::Settled => McpConnectState::Connected,
            ReceiptStatus::Unprovable => McpConnectState::Unprovable,
            ReceiptStatus::Rejected | ReceiptStatus::Accepted => McpConnectState::Failed,
        };
        let summary = (state == McpConnectState::Connected)
            .then(|| self.summary_for(&receipt))
            .transpose()?
            .flatten();
        Ok(McpConnectResult {
            state,
            receipt: Some(receipt),
            summary,
        })
    }

    fn transition(
        &self,
        id: &str,
        expected: McpConnectorPhase,
        next: McpConnectorPhase,
    ) -> Result<bool, RuntimeError> {
        Ok(matches!(
            self.ledger
                .replace_mcp_connector_phase(id, expected, next)?,
            McpConnectorUpdate::Applied(_)
        ))
    }

    fn settle(
        &self,
        receipt: &Receipt,
        state: McpConnectState,
        summary: Option<McpConnectionSummary>,
    ) -> Result<McpConnectResult, RuntimeError> {
        let terminal = Event {
            cursor: 0,
            event_id: terminal_event_id(&receipt.receipt_id),
            receipt_id: receipt.receipt_id.clone(),
            host_epoch: receipt.host_epoch,
            kind: terminal_kind(state).into(),
            payload: summary_payload(summary),
        };
        let receipt = self.ledger.settle_receipt(
            &receipt.idempotency_key,
            receipt_status(state),
            &terminal,
        )?;
        Ok(McpConnectResult {
            state,
            receipt: Some(receipt),
            summary,
        })
    }

    fn summary_for(&self, receipt: &Receipt) -> Result<Option<McpConnectionSummary>, RuntimeError> {
        let Some(event) = self
            .ledger
            .find_event(&terminal_event_id(&receipt.receipt_id))?
        else {
            return Ok(None);
        };
        Ok(event
            .payload
            .get("toolCount")
            .and_then(serde_json::Value::as_u64)
            .map(|tool_count| McpConnectionSummary {
                tool_count: u32::try_from(tool_count).unwrap_or(u32::MAX),
            }))
    }
}

fn valid_request(request: &McpConnectRequest) -> bool {
    request.connector.phase == McpConnectorPhase::Requested
        && request.connector.tool_source_id == request.lease.tool_source_id
        && request.host_epoch == request.lease.host_epoch
}

fn command_for(request: &McpConnectRequest) -> Command {
    Command {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        host_epoch: request.host_epoch,
        kind: "mcpConnect".into(),
        payload: serde_json::json!({
            "connectorId": request.connector.connector_id, "workspaceId": request.connector.workspace_id,
            "toolSourceId": request.connector.tool_source_id,
        }),
    }
}

fn accepted_event(command: &Command) -> Event {
    Event {
        cursor: 0,
        event_id: format!("{}:mcp-connect-accepted", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "mcpConnectAccepted".into(),
        payload: command.payload.clone(),
    }
}

fn terminal_event_id(receipt_id: &ReceiptId) -> String {
    format!("{}:mcp-connect-terminal", receipt_id.0)
}

const fn receipt_status(state: McpConnectState) -> ReceiptStatus {
    match state {
        McpConnectState::Connected => ReceiptStatus::Settled,
        McpConnectState::Unprovable => ReceiptStatus::Unprovable,
        McpConnectState::DeniedObserver | McpConnectState::Failed | McpConnectState::Rejected => {
            ReceiptStatus::Rejected
        }
    }
}

const fn terminal_kind(state: McpConnectState) -> &'static str {
    match state {
        McpConnectState::Connected => "mcpConnectCompleted",
        McpConnectState::Unprovable => "mcpConnectUnprovable",
        McpConnectState::DeniedObserver => "mcpConnectDeniedObserver",
        McpConnectState::Failed => "mcpConnectFailed",
        McpConnectState::Rejected => "mcpConnectRejected",
    }
}

fn summary_payload(summary: Option<McpConnectionSummary>) -> serde_json::Value {
    summary.map_or_else(
        || serde_json::json!({}),
        |value| serde_json::json!({ "toolCount": value.tool_count }),
    )
}
