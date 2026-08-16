use gent_ports::{AgentChatLedger, ConversationLedger, Ledger};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, ConversationRecord, HostEpoch,
    ReceiptId,
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
                "SELECT 1 FROM schema_migrations WHERE version = 21",
                [],
                |_| Ok(()),
            )
            .is_ok()
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
