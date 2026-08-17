use gent_ports::{AgentChatLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId,
};

fn ledger() -> (SqliteLedger, AgentChatConversationId) {
    let ledger = SqliteLedger::in_memory().unwrap();
    let conversation_id = AgentChatConversationId("conversation-1".into());
    ledger
        .create_agent_chat_conversation(&AgentChatConversationCreate {
            receipt_id: ReceiptId("create-receipt".into()),
            idempotency_key: "create-key".into(),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            run_id: AgentChatRunId("run-1".into()),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
        })
        .unwrap();
    (ledger, conversation_id)
}

fn prompt(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    request_id: &str,
    disposition: AgentChatPromptDisposition,
) -> gent_types::AgentChatPromptSaved {
    ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(request_id.into()),
            receipt_id: ReceiptId(format!("receipt-{request_id}")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition,
            text: format!("prompt-{request_id}"),
        })
        .unwrap()
}

#[test]
fn only_send_prompts_enter_a_single_owner_durable_outbox() {
    let (ledger, conversation_id) = ledger();
    let send = prompt(
        &ledger,
        &conversation_id,
        "send",
        AgentChatPromptDisposition::Send,
    );
    let _queued = prompt(
        &ledger,
        &conversation_id,
        "queued",
        AgentChatPromptDisposition::Queue,
    );
    let claimed = ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.message, send.message);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
    ledger
        .release_agent_chat_prompt_dispatch(&send.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    assert_eq!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .unwrap()
            .message
            .message_id,
        send.message.message_id
    );
}

#[test]
fn settlement_and_epoch_fences_prevent_duplicate_or_stale_provider_delivery() {
    let (ledger, conversation_id) = ledger();
    let send = prompt(
        &ledger,
        &conversation_id,
        "send",
        AgentChatPromptDisposition::Send,
    );
    let _ = ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap();
    assert!(
        ledger
            .settle_agent_chat_prompt_dispatch(&send.message.message_id, "daemon-b", HostEpoch(1))
            .is_err()
    );
    ledger
        .settle_agent_chat_prompt_dispatch(&send.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}
