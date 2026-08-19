use gent_ports::{
    AgentChatCompactionLedger, AgentChatLedger, AgentChatWorkspaceLedger, ConversationLedger,
    Ledger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, ConversationRecord, Event, HostEpoch,
    ReceiptId, WorkspaceRecord,
};
use rusqlite::Connection;

fn create(key: &str) -> AgentChatConversationCreate {
    AgentChatConversationCreate {
        receipt_id: ReceiptId(format!("receipt-{key}")),
        idempotency_key: format!("key-{key}"),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId(format!("conversation-{key}")),
        run_id: AgentChatRunId(format!("run-{key}")),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Plan,
        },
    }
}

#[test]
fn creates_conversation_root_run_selection_and_settled_receipt_together() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("agent-chat.db");
    let ledger = SqliteLedger::open(&path).unwrap();
    let command = create("one");

    let result = ledger.create_agent_chat_conversation(&command).unwrap();

    assert_eq!(result.conversation_id, command.conversation_id);
    assert_eq!(result.run_id, command.run_id);
    assert_eq!(result.receipt.status, gent_types::ReceiptStatus::Settled);
    assert_eq!(
        ledger.list_conversations().unwrap(),
        vec![gent_types::ConversationListItem {
            conversation_id: "conversation-one".into(),
            run_count: 1,
        }]
    );
    assert_eq!(
        ledger.list_conversation_runs("conversation-one").unwrap()[0].provider,
        "codex"
    );
    drop(ledger);

    let connection = Connection::open(path).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT provider || ':' || model || ':' || effort || ':' || mode FROM agent_chat_conversations WHERE conversation_id = 'conversation-one'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "codex:gpt-5.6:high:plan"
    );
    assert!(
        connection
            .query_row(
                "SELECT 1 FROM gent_schema WHERE identity = 'gent-fresh-schema-v7'",
                [],
                |_| Ok(()),
            )
            .is_ok()
    );
}

#[test]
fn workspace_bound_creation_is_atomic_and_retry_safe() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let create = create("workspace");
    let workspace = WorkspaceRecord {
        workspace_id: "workspace-1".into(),
        canonical_path: "/verified/workspace".into(),
    };

    ledger
        .create_agent_chat_conversation_in_workspace(&create, &workspace)
        .unwrap();
    assert_eq!(
        ledger
            .create_agent_chat_conversation_in_workspace(&create, &workspace)
            .unwrap()
            .run_id,
        create.run_id
    );
    assert_eq!(
        ledger
            .agent_chat_workspace_for_run(&create.conversation_id.0, &create.run_id.0)
            .unwrap(),
        workspace
    );
    let mut conflicting = workspace;
    conflicting.canonical_path = "/other/workspace".into();
    assert!(
        ledger
            .create_agent_chat_conversation_in_workspace(&create, &conflicting)
            .is_err()
    );
}

#[test]
fn exact_retry_returns_the_same_owned_result_without_duplicate_rows() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let command = create("retry");
    let first = ledger.create_agent_chat_conversation(&command).unwrap();
    let second = ledger.create_agent_chat_conversation(&command).unwrap();

    assert_eq!(first, second);
    assert_eq!(ledger.list_conversations().unwrap().len(), 1);
    assert_eq!(
        ledger
            .list_conversation_runs("conversation-retry")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn retries_cannot_rebind_the_idempotency_key_or_receipt_id() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let command = create("original");
    ledger.create_agent_chat_conversation(&command).unwrap();

    let mut different_payload = command.clone();
    different_payload.run_id = AgentChatRunId("run-other".into());
    assert!(
        ledger
            .create_agent_chat_conversation(&different_payload)
            .is_err()
    );

    let mut different_key = create("different");
    different_key.receipt_id = command.receipt_id.clone();
    assert!(
        ledger
            .create_agent_chat_conversation(&different_key)
            .is_err()
    );
    assert_eq!(ledger.list_conversations().unwrap().len(), 1);
}

#[test]
fn failed_root_run_insert_leaves_no_partial_conversation_or_receipt() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_conversation_run(
            &ConversationRecord {
                conversation_id: "existing".into(),
            },
            &gent_ports::RunRecord {
                run_id: "run-taken".into(),
                parent_run_id: None,
                provider: "codex".into(),
            },
        )
        .unwrap();
    let mut command = create("atomic");
    command.run_id = AgentChatRunId("run-taken".into());

    assert!(ledger.create_agent_chat_conversation(&command).is_err());
    assert!(
        ledger
            .find_conversation("conversation-atomic")
            .unwrap()
            .is_none()
    );
    assert_eq!(ledger.list_conversations().unwrap().len(), 1);
}

#[test]
fn closed_ingress_rejects_before_creating_any_agent_chat_state() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();

    assert!(
        ledger
            .create_agent_chat_conversation(&create("closed"))
            .is_err()
    );
    assert!(
        ledger
            .find_conversation("conversation-closed")
            .unwrap()
            .is_none()
    );
}

#[test]
fn compaction_pages_are_filtered_canonical_events_not_a_second_fact_store() {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation(&create("compaction"))
        .unwrap();
    let event = |event_id: &str, kind: &str, run_id: &str| Event {
        cursor: 0,
        event_id: event_id.into(),
        receipt_id: ReceiptId("provider:run-compaction".into()),
        host_epoch: HostEpoch(1),
        kind: kind.into(),
        payload: serde_json::json!({
            "runId": run_id,
            "compaction": {"type":"started","eventId":event_id,"turnId":"turn-1"}
        }),
    };
    ledger
        .append_event(&event(
            "compaction-one",
            "agentChatCompaction",
            "run-compaction",
        ))
        .unwrap();
    ledger
        .append_event(&event("other-run", "agentChatCompaction", "run-other"))
        .unwrap();
    ledger
        .append_event(&event("other-kind", "providerLifecycle", "run-compaction"))
        .unwrap();

    let page = ledger
        .read_agent_chat_compaction_page("run-compaction", 0, 10)
        .unwrap();
    assert_eq!(page.next_after_cursor, None);
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event_id, "compaction-one");
    assert_eq!(
        ledger.find_event("compaction-one").unwrap(),
        Some(page.events[0].clone())
    );
}
