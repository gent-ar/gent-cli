use gent_ports::{ConversationLedger, ConversationPromptLedger};
use gent_runtime::{
    AgentChatConversationAuthority, AgentChatConversationRequest, AgentChatConversationResult,
    AgentChatConversationService, AgentChatPromptAuthority, AgentChatPromptRequest,
    AgentChatPromptResult, AgentChatPromptService,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatPromptDisposition,
    AgentChatProvider, AgentChatRequestId, AgentChatSelection, HostEpoch, ReceiptId, ReceiptStatus,
};

fn selection() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Claude,
        model: "haiku".into(),
        effort: AgentChatEffort::Low,
        mode: AgentChatMode::Ask,
    }
}

fn conversation(ledger: SqliteLedger) -> AgentChatConversationId {
    let created =
        AgentChatConversationService::new(ledger, AgentChatConversationAuthority::Approved)
            .create(&AgentChatConversationRequest {
                request_id: AgentChatRequestId("conversation-request".into()),
                receipt_id: ReceiptId("conversation-receipt".into()),
                host_epoch: HostEpoch(1),
                selection: selection(),
            })
            .unwrap();
    let AgentChatConversationResult::Created(created) = created else {
        panic!("approved setup must create a conversation");
    };
    created.conversation_id
}

fn prompt(conversation_id: AgentChatConversationId) -> AgentChatPromptRequest {
    AgentChatPromptRequest {
        request_id: AgentChatRequestId("prompt-request".into()),
        receipt_id: ReceiptId("prompt-receipt".into()),
        host_epoch: HostEpoch(1),
        conversation_id,
        disposition: AgentChatPromptDisposition::Send,
        text: "hello".into(),
    }
}

#[test]
fn observer_returns_before_receipt_or_prompt_write() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = conversation(ledger.clone());
    let result = AgentChatPromptService::new(ledger.clone(), AgentChatPromptAuthority::Observer)
        .submit(&prompt(conversation_id.clone()))
        .unwrap();

    assert_eq!(result, AgentChatPromptResult::DeniedObserver);
    let runs = ledger.list_conversation_runs(&conversation_id.0).unwrap();
    assert_eq!(runs.len(), 1);
    assert!(
        ledger
            .list_run_messages(&runs[0].run_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn approved_authority_returns_the_same_settled_message_and_run_on_retry() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = conversation(ledger.clone());
    let service = AgentChatPromptService::new(ledger.clone(), AgentChatPromptAuthority::Approved);
    let request = prompt(conversation_id);
    let first = service.submit(&request).unwrap();
    let second = service.submit(&request).unwrap();

    assert_eq!(first, second);
    let AgentChatPromptResult::Saved(saved) = first else {
        panic!("approved request must persist a prompt");
    };
    assert_eq!(saved.receipt.status, ReceiptStatus::Settled);
    assert_eq!(saved.message.text, "hello");
    assert_eq!(ledger.list_run_messages(&saved.run_id.0).unwrap().len(), 1);
}

#[test]
fn queue_disposition_is_persisted_without_any_provider_effect() {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = conversation(ledger.clone());
    let service = AgentChatPromptService::new(ledger, AgentChatPromptAuthority::Approved);
    let mut request = prompt(conversation_id);
    request.disposition = AgentChatPromptDisposition::Queue;
    assert!(matches!(
        service.submit(&request).unwrap(),
        AgentChatPromptResult::Saved(saved) if saved.disposition == AgentChatPromptDisposition::Queue
    ));
}
