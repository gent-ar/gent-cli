use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatWorkspaceLedger, Ledger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, HostEpoch, ReceiptId, WorkspaceRecord,
};

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
) -> gent_types::AgentChatPromptSaved {
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
    if disposition == AgentChatPromptDisposition::Send {
        ledger
            .release_agent_chat_prompt_after_readiness(
                &saved.message.message_id,
                &saved.run_id,
                HostEpoch(1),
            )
            .unwrap();
    }
    saved
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
        .release_agent_chat_prompt_claim(&send.message.message_id, "daemon-a", HostEpoch(1))
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
fn claim_excluding_active_runs_advances_to_another_conversation() {
    let (ledger, first_conversation) = ledger();
    let second_conversation = AgentChatConversationId("conversation-2".into());
    ledger
        .create_agent_chat_conversation_in_workspace(
            &AgentChatConversationCreate {
                receipt_id: ReceiptId("create-receipt-2".into()),
                idempotency_key: "create-key-2".into(),
                host_epoch: HostEpoch(1),
                conversation_id: second_conversation.clone(),
                run_id: AgentChatRunId("run-2".into()),
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
    let blocked = prompt(
        &ledger,
        &first_conversation,
        "blocked",
        AgentChatPromptDisposition::Send,
    );
    let available = prompt(
        &ledger,
        &second_conversation,
        "available",
        AgentChatPromptDisposition::Send,
    );

    let claimed = ledger
        .claim_agent_chat_prompt_dispatch_excluding_runs(
            "daemon-a",
            HostEpoch(1),
            AgentChatProvider::Codex,
            &[blocked.run_id],
        )
        .unwrap()
        .unwrap();

    assert_eq!(claimed.message, available.message);
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
        .begin_agent_chat_prompt_launch(&send.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(&send.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
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

#[test]
fn successor_replays_only_known_prelaunch_work() {
    let (ledger, conversation_id) = ledger();
    let send = prompt(
        &ledger,
        &conversation_id,
        "claimed",
        AgentChatPromptDisposition::Send,
    );
    ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    assert_eq!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-b", HostEpoch(2), AgentChatProvider::Codex)
            .unwrap()
            .unwrap()
            .message,
        send.message
    );
}

#[test]
fn successor_never_replays_an_ambiguous_launch() {
    let (ledger, conversation_id) = ledger();
    let send = prompt(
        &ledger,
        &conversation_id,
        "launching",
        AgentChatPromptDisposition::Send,
    );
    ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap();
    ledger
        .begin_agent_chat_prompt_launch(&send.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    ledger.fence_and_open(HostEpoch(1)).unwrap();
    ledger
        .recover_agent_chat_prompt_dispatches(HostEpoch(2))
        .unwrap();
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-b", HostEpoch(2), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}

#[test]
fn daemon_marks_an_ambiguous_launch_unprovable_before_restart() {
    let (ledger, conversation_id) = ledger();
    let send = prompt(
        &ledger,
        &conversation_id,
        "unprovable",
        AgentChatPromptDisposition::Send,
    );
    ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap();
    ledger
        .begin_agent_chat_prompt_launch(&send.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    ledger
        .mark_agent_chat_prompt_unprovable(&send.message.message_id, "daemon-a", HostEpoch(1))
        .unwrap();
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
}

#[path = "support/agent_chat_prompt_dispatch_readiness_tests.rs"]
mod readiness_tests;
