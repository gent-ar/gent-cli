use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatQueuedPromptLedger,
    AgentChatWorkspaceLedger, ConversationLedger, Ledger, LedgerError,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, DurableTurnPhase, HostEpoch, ReceiptId,
    TurnTerminalCause, WorkspaceRecord,
};

const DAEMON: &str = "daemon-a";

fn ledger() -> (SqliteLedger, AgentChatConversationId) {
    let ledger = SqliteLedger::in_memory().unwrap();
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

fn prompt(
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

fn active(
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

fn steer(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    receipt: &str,
    prompt: &AgentChatPromptSaved,
) -> Result<(gent_types::Receipt, AgentChatRunId), LedgerError> {
    ledger.steer_queued_agent_chat_prompt(
        &ReceiptId(receipt.into()),
        HostEpoch(1),
        conversation_id,
        &prompt.message.message_id,
    )
}

fn restart(ledger: &SqliteLedger) {
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
}

#[test]
fn a_steer_accepted_but_never_written_is_delivered_exactly_once_after_a_restart() {
    let (ledger, conversation_id) = ledger();
    active(&ledger, &conversation_id);
    let queued = prompt(
        &ledger,
        &conversation_id,
        "queued",
        AgentChatPromptDisposition::Queue,
    );
    steer(&ledger, &conversation_id, "steer-1", &queued).unwrap();
    restart(&ledger);
    let delivered = ledger
        .claim_agent_chat_prompt_dispatch("daemon-b", HostEpoch(2), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    assert_eq!(delivered.message.message_id, queued.message.message_id);
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-b", HostEpoch(2), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
    assert_eq!(terminal_cause(&ledger, &queued), RecordedTerminal::Absent);
}

#[test]
fn a_possibly_delivered_steer_fails_with_a_typed_unprovable_delivery_cause() {
    let (ledger, conversation_id) = ledger();
    let send = active(&ledger, &conversation_id);
    let queued = prompt(
        &ledger,
        &conversation_id,
        "queued",
        AgentChatPromptDisposition::Queue,
    );
    steer(&ledger, &conversation_id, "steer-1", &queued).unwrap();
    ledger
        .claim_steered_agent_chat_prompt(DAEMON, HostEpoch(1), &send.run_id)
        .unwrap()
        .unwrap();
    restart(&ledger);
    assert_eq!(
        ledger
            .find_turn(&queued.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Failed
    );
    assert_eq!(
        terminal_cause(&ledger, &queued),
        RecordedTerminal::Caused(TurnTerminalCause::DeliveryUnprovable)
    );
    assert_eq!(terminal_cause(&ledger, &send), RecordedTerminal::Untyped);
}

#[path = "support/terminal_cause.rs"]
mod terminal_cause_support;
use terminal_cause_support::{RecordedTerminal, terminal_cause};
