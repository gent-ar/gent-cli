use gent_ports::{AgentChatLedger, AgentChatPromptLedger, ConversationPromptLedger, Ledger};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDelivery, AgentChatPromptDisposition, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId, ReceiptStatus,
};

fn conversation() -> AgentChatConversationCreate {
    AgentChatConversationCreate {
        receipt_id: ReceiptId("receipt-conversation".into()),
        idempotency_key: "conversation-key".into(),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        run_id: AgentChatRunId("run-1".into()),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "haiku".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Ask,
        },
    }
}

fn prompt(request_id: &str, text: &str) -> AgentChatPromptCreate {
    AgentChatPromptCreate {
        request_id: AgentChatRequestId(request_id.into()),
        receipt_id: ReceiptId(format!("receipt-{request_id}")),
        host_epoch: HostEpoch(1),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        disposition: AgentChatPromptDisposition::Send,
        text: text.into(),
    }
}

fn ledger() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation(&conversation())
        .unwrap();
    ledger
}

#[test]
fn saves_message_turn_receipt_and_ordinal_in_one_durable_result() {
    let ledger = ledger();
    let saved = ledger
        .save_agent_chat_prompt(&prompt("one", "hello"))
        .unwrap();

    assert_eq!(saved.receipt.status, ReceiptStatus::Settled);
    assert_eq!(saved.run_id.0, "run-1");
    assert_eq!(saved.delivery, AgentChatPromptDelivery::AwaitingProvider);
    assert_eq!(saved.message.sequence, 1);
    assert_eq!(saved.message.text, "hello");
    assert_eq!(
        ledger.list_run_messages("run-1").unwrap(),
        vec![saved.message]
    );
}

#[test]
fn exact_retry_returns_the_same_settled_message_without_a_duplicate_turn() {
    let ledger = ledger();
    let request = prompt("retry", "one immutable prompt");
    let first = ledger.save_agent_chat_prompt(&request).unwrap();
    let second = ledger.save_agent_chat_prompt(&request).unwrap();

    assert_eq!(first, second);
    assert_eq!(ledger.list_run_messages("run-1").unwrap().len(), 1);
    let mut conflict = request;
    conflict.text = "changed".into();
    assert!(ledger.save_agent_chat_prompt(&conflict).is_err());
}

#[test]
fn unknown_conversation_or_closed_ingress_leaves_existing_run_content_unchanged() {
    let ledger = ledger();
    let mut missing = prompt("missing", "never saved");
    missing.conversation_id = AgentChatConversationId("unknown".into());
    assert!(ledger.save_agent_chat_prompt(&missing).is_err());
    assert!(ledger.list_run_messages("run-1").unwrap().is_empty());

    ledger.close_ingress(HostEpoch(1)).unwrap();
    assert!(
        ledger
            .save_agent_chat_prompt(&prompt("closed", "never saved"))
            .is_err()
    );
    assert!(ledger.list_run_messages("run-1").unwrap().is_empty());
}
