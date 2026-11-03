use gent_ports::{AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId, WorkspaceRecord,
};

pub const DAEMON: &str = "daemon-a";

pub fn ledger() -> (SqliteLedger, AgentChatConversationId) {
    let ledger = SqliteLedger::in_memory().unwrap();
    populate(ledger)
}

pub fn populate(ledger: SqliteLedger) -> (SqliteLedger, AgentChatConversationId) {
    let conversation_id = AgentChatConversationId("conversation-1".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
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
            },
            &WorkspaceRecord {
                workspace_id: "workspace-1".into(),
                canonical_path: "/workspace".into(),
            },
        )
        .unwrap();
    (ledger, conversation_id)
}

pub fn prompt(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    request_id: &str,
    disposition: AgentChatPromptDisposition,
) -> AgentChatPromptSaved {
    let saved = ledger
        .save_agent_chat_prompt(&AgentChatPromptCreate {
            request_id: AgentChatRequestId(request_id.into()),
            receipt_id: ReceiptId(format!("receipt-{request_id}")),
            host_epoch: HostEpoch(1),
            conversation_id: conversation_id.clone(),
            disposition,
            text: format!("prompt-{request_id}"),
            attachment_ids: vec![],
            tool_source_ids: vec![],
        })
        .unwrap();
    ledger
        .release_agent_chat_prompt_after_readiness(
            &saved.message.message_id,
            &saved.run_id,
            HostEpoch(1),
        )
        .unwrap();
    saved
}

pub fn active(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
) -> AgentChatPromptSaved {
    let send = prompt(
        ledger,
        conversation_id,
        "send",
        AgentChatPromptDisposition::Send,
    );
    ledger
        .claim_agent_chat_prompt_dispatch(DAEMON, HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    let id = &send.message.message_id;
    ledger
        .begin_agent_chat_prompt_launch(id, DAEMON, HostEpoch(1))
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(id, DAEMON, HostEpoch(1))
        .unwrap();
    send
}
