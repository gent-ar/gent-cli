use super::*;
use gent_ports::{
    AgentChatWorkspaceLedger, ForgeConnectorLedger, Ledger, McpConnectorLedger, ToolSourceLedger,
};
use gent_types::*;

fn fixture() -> (
    SqliteLedger,
    Command,
    ToolSourceRecord,
    ForgeConnectorRecord,
) {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create".into()),
                idempotency_key: "create".into(),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation".into()),
                run_id: AgentChatRunId("run".into()),
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "fixture".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
            },
            &WorkspaceRecord {
                workspace_id: "workspace".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    let source = ToolSourceRecord {
        tool_source_id: "source".into(),
        workspace_id: "workspace".into(),
        kind: ToolSourceKind::McpServer,
        source_name: "example-forge".into(),
        declared_tools: vec!["read".into()],
    };
    let connector = ForgeConnectorRecord {
        connector_id: "connector".into(),
        workspace_id: source.workspace_id.clone(),
        tool_source_id: source.tool_source_id.clone(),
        name: "Example".into(),
        description: "Synthetic connector".into(),
        category: "documents".into(),
        phase: McpConnectorPhase::Requested,
        declared_tools: source.declared_tools.clone(),
        discovered_tools: vec![],
        enabled: false,
    };
    let command = Command {
        receipt_id: ReceiptId("register".into()),
        idempotency_key: "register".into(),
        host_epoch: HostEpoch(1),
        kind: "managedMcpRegister".into(),
        payload: serde_json::json!({
            "connectorId":"connector","toolSourceId":"source","workspaceId":"workspace",
            "conversationId":"conversation","runId":"run","artifactDigest":"a".repeat(64),"registrationDigest":"b".repeat(64)
        }),
    };
    (ledger, command, source, connector)
}

fn accept(ledger: &SqliteLedger, command: &Command) {
    ledger
        .claim_command(
            command,
            &Event {
                cursor: 0,
                event_id: "register-accepted".into(),
                receipt_id: command.receipt_id.clone(),
                host_epoch: command.host_epoch,
                kind: "managedMcpRegisterAccepted".into(),
                payload: command.payload.clone(),
            },
        )
        .unwrap();
}

fn no_rows(ledger: &SqliteLedger) {
    assert!(ledger.find_tool_source("source").unwrap().is_none());
    assert!(ledger.find_mcp_connector("connector").unwrap().is_none());
    assert!(ledger.find_forge_connector("connector").unwrap().is_none());
}

#[test]
fn atomic_draft_does_not_claim_discovery_or_execution() {
    let (ledger, command, source, connector) = fixture();
    accept(&ledger, &command);
    let receipt = commit(
        &ledger,
        &command,
        &source,
        &connector,
        "conversation",
        "run",
    )
    .unwrap();
    assert_eq!(receipt.status, ReceiptStatus::Settled);
    assert_eq!(ledger.find_tool_source("source").unwrap(), Some(source));
    let saved = ledger.find_forge_connector("connector").unwrap().unwrap();
    assert_eq!(saved.phase, McpConnectorPhase::Requested);
    assert!(!saved.enabled);
    assert!(saved.discovered_tools.is_empty());
    assert!(
        ledger
            .find_event("managed-mcp-registration-prepared:register")
            .unwrap()
            .is_some()
    );
    assert!(
        commit(
            &ledger,
            &command,
            &ledger.find_tool_source("source").unwrap().unwrap(),
            &connector,
            "conversation",
            "run"
        )
        .is_err()
    );
}

#[test]
fn exact_acceptance_and_current_selected_run_are_required_without_partial_rows() {
    let (ledger, command, source, connector) = fixture();
    assert!(
        commit(
            &ledger,
            &command,
            &source,
            &connector,
            "conversation",
            "run"
        )
        .is_err()
    );
    no_rows(&ledger);
    accept(&ledger, &command);
    let mut changed = command.clone();
    changed.payload["registrationDigest"] = serde_json::json!("c".repeat(64));
    assert!(
        commit(
            &ledger,
            &changed,
            &source,
            &connector,
            "conversation",
            "run"
        )
        .is_err()
    );
    no_rows(&ledger);
    assert!(
        commit(
            &ledger,
            &command,
            &source,
            &connector,
            "conversation",
            "other"
        )
        .is_err()
    );
    no_rows(&ledger);
}

#[test]
fn private_fields_and_reserved_namespace_cannot_enter_public_draft() {
    let (ledger, mut command, source, connector) = fixture();
    command.payload["credentials"] = serde_json::json!({"TOKEN":"synthetic-sensitive"});
    accept(&ledger, &command);
    assert!(
        commit(
            &ledger,
            &command,
            &source,
            &connector,
            "conversation",
            "run"
        )
        .is_err()
    );
    no_rows(&ledger);
    let (ledger, command, mut source, connector) = fixture();
    source.source_name = "gent-forge".into();
    accept(&ledger, &command);
    assert!(
        commit(
            &ledger,
            &command,
            &source,
            &connector,
            "conversation",
            "run"
        )
        .is_err()
    );
    no_rows(&ledger);
}

#[test]
fn connector_collision_rolls_back_new_source_insert() {
    let (ledger, command, mut source, mut connector) = fixture();
    ledger.create_tool_source(&source).unwrap();
    ledger
        .create_mcp_connector(&McpConnectorRecord {
            connector_id: connector.connector_id.clone(),
            workspace_id: connector.workspace_id.clone(),
            tool_source_id: source.tool_source_id.clone(),
            phase: McpConnectorPhase::Requested,
        })
        .unwrap();
    ledger.create_forge_connector(&connector).unwrap();
    source.tool_source_id = "new-source".into();
    connector.tool_source_id = source.tool_source_id.clone();
    let mut command = command;
    command.payload["toolSourceId"] = serde_json::json!("new-source");
    accept(&ledger, &command);
    assert!(
        commit(
            &ledger,
            &command,
            &source,
            &connector,
            "conversation",
            "run"
        )
        .is_err()
    );
    assert!(ledger.find_tool_source("new-source").unwrap().is_none());
    assert_eq!(
        ledger
            .find_mcp_connector("connector")
            .unwrap()
            .unwrap()
            .tool_source_id,
        "source"
    );
}
