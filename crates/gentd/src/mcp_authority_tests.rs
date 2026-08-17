use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use gent_mcp::{ConnectorDefinition, ConnectorId, ToolDefinition, ToolName, ToolRegistry};
use gent_ports::{
    McpConnectOperation, McpConnectionSummary, McpConnectorError, McpConnectorExecutor,
    ToolSourceLedger, WorkspaceLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    HostEpoch, McpConnectorPhase, McpConnectorRecord, ReceiptId, ToolSourceKind, ToolSourceRecord,
    WorkspaceRecord,
};

use super::mcp_authority::{McpAuthorityError, McpAuthorityRuntime, registry_sha256};
use super::{
    AuthorityProfileConfig, McpApproval, McpRequest, ValidatedAuthorityProfile,
    shipped_observer_profile,
};

#[derive(Clone, Debug, Default)]
struct Executor(Arc<AtomicUsize>);

impl McpConnectorExecutor for Executor {
    fn connect(&self, _: &McpConnectOperation) -> Result<McpConnectionSummary, McpConnectorError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(McpConnectionSummary { tool_count: 1 })
    }
}

fn registry() -> ToolRegistry {
    ToolRegistry::build(vec![ConnectorDefinition {
        id: ConnectorId::parse("example-mcp").unwrap(),
        tools: vec![ToolDefinition {
            name: ToolName::parse("read").unwrap(),
        }],
    }])
    .unwrap()
}

fn approved(registry: &ToolRegistry) -> ValidatedAuthorityProfile {
    AuthorityProfileConfig {
        mcp: McpRequest::Approved(McpApproval {
            evidence_reference: "mcp-live-evidence".into(),
            registry_sha256: registry_sha256(registry),
        }),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .unwrap()
}

fn ledger() -> SqliteLedger {
    ledger_with_tools(vec!["read".into()])
}

fn ledger_with_tools(declared_tools: Vec<String>) -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_workspace(&WorkspaceRecord {
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
            declared_tools,
        })
        .unwrap();
    ledger
}

fn request(id: &str) -> gent_runtime::McpConnectRequest {
    gent_runtime::McpConnectRequest {
        receipt_id: ReceiptId(format!("receipt-{id}")),
        idempotency_key: id.into(),
        host_epoch: HostEpoch(1),
        connector: McpConnectorRecord {
            connector_id: format!("connector-{id}"),
            workspace_id: "workspace".into(),
            tool_source_id: "source".into(),
            phase: McpConnectorPhase::Requested,
        },
        lease: gent_ports::McpConnectorLease {
            tool_source_id: "source".into(),
            lease_token: format!("lease-{id}"),
            host_epoch: HostEpoch(1),
        },
    }
}

#[test]
fn observer_profile_rejects_before_an_executor_can_be_bound() {
    let executor = Executor::default();
    let result =
        McpAuthorityRuntime::new(shipped_observer_profile(), ledger(), registry(), executor);
    assert!(matches!(result, Err(McpAuthorityError::ObserverProfile)));
}

#[test]
fn approved_registry_binds_executor_after_durable_fences() {
    let registry = registry();
    let executor = Executor::default();
    let calls = Arc::clone(&executor.0);
    let runtime =
        McpAuthorityRuntime::new(approved(&registry), ledger(), registry, executor).unwrap();
    let result = runtime.connect(&request("approved")).unwrap();
    assert_eq!(result.state, gent_runtime::McpConnectState::Connected);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn mismatched_registry_cannot_bind_an_executor() {
    let registry = registry();
    let wrong = AuthorityProfileConfig {
        mcp: McpRequest::Approved(McpApproval {
            evidence_reference: "mcp-live-evidence".into(),
            registry_sha256: "a".repeat(64),
        }),
        ..AuthorityProfileConfig::default()
    }
    .validate()
    .unwrap();
    let result = McpAuthorityRuntime::new(wrong, ledger(), registry, Executor::default());
    assert!(matches!(result, Err(McpAuthorityError::RegistryMismatch)));
}

#[test]
fn an_unregistered_source_declaration_settles_without_calling_the_executor() {
    let registry = registry();
    let executor = Executor::default();
    let calls = Arc::clone(&executor.0);
    let runtime = McpAuthorityRuntime::new(
        approved(&registry),
        ledger_with_tools(vec!["write".into()]),
        registry,
        executor,
    )
    .unwrap();
    let result = runtime.connect(&request("unregistered")).unwrap();
    assert_eq!(result.state, gent_runtime::McpConnectState::Failed);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        result.receipt.unwrap().status,
        gent_types::ReceiptStatus::Rejected
    );
}
