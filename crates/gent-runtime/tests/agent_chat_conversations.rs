use gent_ports::ConversationLedger;
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationRequest, AgentChatConversationResult,
    AgentChatConversationService,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId, AgentChatSelection,
    HostEpoch, ReceiptId, ReceiptStatus, WorkspaceRecord,
};

fn request(id: &str) -> AgentChatConversationRequest {
    AgentChatConversationRequest {
        request_id: AgentChatRequestId(id.into()),
        receipt_id: ReceiptId(format!("receipt-{id}")),
        host_epoch: HostEpoch(1),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Claude,
            model: "haiku".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Ask,
        },
        workspace: WorkspaceRecord {
            workspace_id: "workspace-1".into(),
            canonical_path: "/workspace".into(),
        },
    }
}

#[test]
fn observer_denies_before_any_receipt_or_conversation_write() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let result =
        AgentChatConversationService::new(ledger.clone(), AgentChatConversationAuthority::Observer)
            .create(&request("observer"))
            .unwrap();

    assert_eq!(result, AgentChatConversationResult::DeniedObserver);
    assert!(ledger.list_conversations().unwrap().is_empty());
}

#[test]
fn approved_authority_creates_retry_stable_public_conversation_and_run() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let service =
        AgentChatConversationService::new(ledger.clone(), AgentChatConversationAuthority::Approved);
    let first = service.create(&request("stable")).unwrap();
    let second = service.create(&request("stable")).unwrap();

    assert_eq!(first, second);
    let AgentChatConversationResult::Created(created) = first else {
        panic!("approved creation must produce durable identities");
    };
    assert_eq!(created.receipt.status, ReceiptStatus::Settled);
    assert_eq!(ledger.list_conversations().unwrap().len(), 1);
    assert_eq!(
        ledger
            .list_conversation_runs(&created.conversation_id.0)
            .unwrap()[0]
            .run_id,
        created.run_id.0
    );
}

#[test]
fn distinct_request_correlation_allocates_distinct_public_identities() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let service =
        AgentChatConversationService::new(ledger, AgentChatConversationAuthority::Approved);
    let AgentChatConversationResult::Created(first) = service.create(&request("one")).unwrap()
    else {
        panic!("first request must create");
    };
    let AgentChatConversationResult::Created(second) = service.create(&request("two")).unwrap()
    else {
        panic!("second request must create");
    };
    assert_ne!(first.conversation_id, second.conversation_id);
    assert_ne!(first.run_id, second.run_id);
}
