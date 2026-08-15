use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use gent_ports::{
    Ledger, McpConnectOperation, McpConnectionSummary, McpConnectorError, McpConnectorExecutor,
    McpConnectorLedger, ReceiptClaim, ToolSourceLedger, WorkspaceLedger,
};
use gent_runtime::{McpConnectRequest, McpConnectState, McpConnectorService};
use gent_store::SqliteLedger;
use gent_types::{
    Command, Event, HostEpoch, McpConnectorPhase, McpConnectorRecord, ReceiptId, ToolSourceKind,
    ToolSourceRecord,
};
use serde_json::json;

#[derive(Clone, Debug, Default)]
struct FakeMcp {
    calls: Arc<AtomicUsize>,
}

impl McpConnectorExecutor for FakeMcp {
    fn connect(
        &self,
        operation: &McpConnectOperation,
    ) -> Result<McpConnectionSummary, McpConnectorError> {
        assert_eq!(operation.source_name, "example-mcp");
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(McpConnectionSummary {
            tool_count: u32::try_from(operation.declared_tools.len()).unwrap(),
        })
    }
}

fn request(key: &str, connector_id: &str) -> McpConnectRequest {
    McpConnectRequest {
        receipt_id: ReceiptId(format!("receipt-{key}")),
        idempotency_key: key.into(),
        host_epoch: HostEpoch(1),
        connector: McpConnectorRecord {
            connector_id: connector_id.into(),
            workspace_id: "workspace".into(),
            tool_source_id: "source".into(),
            phase: McpConnectorPhase::Requested,
        },
        lease: gent_ports::McpConnectorLease {
            tool_source_id: "source".into(),
            lease_token: format!("lease-{key}"),
            host_epoch: HostEpoch(1),
        },
    }
}

fn ledger_with_source() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_workspace(&gent_types::WorkspaceRecord {
            workspace_id: "workspace".into(),
            canonical_path: "/workspace".into(),
        })
        .unwrap();
    ledger
        .create_tool_source(&ToolSourceRecord {
            tool_source_id: "source".into(),
            workspace_id: "workspace".into(),
            kind: ToolSourceKind::McpServer,
            source_name: "example-mcp".into(),
            declared_tools: vec!["example:read".into(), "example:write".into()],
        })
        .unwrap();
    ledger
}

#[test]
fn observer_mode_never_claims_a_receipt_lease_or_connector_effect() {
    let ledger = ledger_with_source();
    let fake = FakeMcp::default();
    let result = McpConnectorService::new(ledger.clone(), fake.clone(), false)
        .connect(&request("observer", "connector-observer"))
        .unwrap();
    assert_eq!(result.state, McpConnectState::DeniedObserver);
    assert!(result.receipt.is_none());
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert!(
        ledger
            .find_mcp_connector("connector-observer")
            .unwrap()
            .is_none()
    );
    assert!(ledger.find_mcp_connector_lease("source").unwrap().is_none());
}

#[test]
fn authority_claims_receipt_and_lease_then_returns_a_durable_idempotent_summary() {
    let ledger = ledger_with_source();
    let fake = FakeMcp::default();
    let service = McpConnectorService::new(ledger.clone(), fake.clone(), true);
    let request = request("success", "connector-success");
    let first = service.connect(&request).unwrap();
    let second = service.connect(&request).unwrap();
    assert_eq!(first.state, McpConnectState::Connected);
    assert_eq!(first, second);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        ledger
            .find_mcp_connector("connector-success")
            .unwrap()
            .unwrap()
            .phase,
        McpConnectorPhase::Ready
    );
}

#[test]
fn a_contended_source_rejects_before_connector_execution() {
    let ledger = ledger_with_source();
    ledger
        .claim_mcp_connector_lease(&gent_ports::McpConnectorLease {
            tool_source_id: "source".into(),
            lease_token: "existing".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    let fake = FakeMcp::default();
    let result = McpConnectorService::new(ledger, fake.clone(), true)
        .connect(&request("contended", "connector-contended"))
        .unwrap();
    assert_eq!(result.state, McpConnectState::Rejected);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn an_accepted_restart_receipt_becomes_unprovable_without_reconnecting() {
    let ledger = ledger_with_source();
    let request = request("restart", "connector-restart");
    let command = Command {
        receipt_id: request.receipt_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        host_epoch: request.host_epoch,
        kind: "mcpConnect".into(),
        payload: json!({ "connectorId": "connector-restart", "workspaceId": "workspace", "toolSourceId": "source" }),
    };
    let accepted = Event {
        cursor: 0,
        event_id: format!("{}:mcp-connect-accepted", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "mcpConnectAccepted".into(),
        payload: command.payload.clone(),
    };
    assert!(matches!(
        ledger.claim_command(&command, &accepted).unwrap(),
        ReceiptClaim::Accepted(_)
    ));
    let fake = FakeMcp::default();
    let result = McpConnectorService::new(ledger, fake.clone(), true)
        .connect(&request)
        .unwrap();
    assert_eq!(result.state, McpConnectState::Unprovable);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}
