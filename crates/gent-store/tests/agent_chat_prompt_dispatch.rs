use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatQueuedPromptLedger,
    AgentChatWorkspaceLedger, ConversationActivityLedger, ConversationLedger, Ledger, LedgerError,
    PendingPermissionLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatDecisionId, AgentChatEffort,
    AgentChatMode, AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatProvider,
    AgentChatRequestId, AgentChatRunId, AgentChatSelection, HostEpoch, PermissionCategory,
    PermissionDecisionBinding, PermissionDecisionRequest, PermissionRequest,
    PermissionRequestDigest, ReceiptId, WorkspaceRecord,
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
    ledger
        .release_agent_chat_prompt_after_readiness(
            &saved.message.message_id,
            &saved.run_id,
            HostEpoch(1),
        )
        .unwrap();
    saved
}

#[test]
fn queued_prompts_wait_behind_the_active_run_and_publish_typed_state() {
    let (ledger, conversation_id) = ledger();
    let send = prompt(
        &ledger,
        &conversation_id,
        "send",
        AgentChatPromptDisposition::Send,
    );
    let queued = prompt(
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
    let initial = ledger
        .read_conversation_activity_page(&conversation_id.0, &send.run_id.0, 0, 10)
        .unwrap();
    assert!(matches!(
        initial.facts.as_slice(),
        [gent_types::ConversationActivityFact::PromptQueued { message_id, .. }]
            if message_id == &queued.message.message_id
    ));
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch_excluding_runs(
                "daemon-a",
                HostEpoch(1),
                AgentChatProvider::Codex,
                std::slice::from_ref(&send.run_id),
            )
            .unwrap()
            .is_none()
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
    let released = ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    assert_eq!(released.message.message_id, queued.message.message_id);
    assert_eq!(released.disposition, AgentChatPromptDisposition::Queue);
    let activity = ledger
        .read_conversation_activity_page(&conversation_id.0, &send.run_id.0, 0, 10)
        .unwrap();
    assert!(matches!(
        activity.facts.as_slice(),
        [
            gent_types::ConversationActivityFact::PromptQueued { .. },
            gent_types::ConversationActivityFact::PromptReleased { message_id, .. }
        ] if message_id == &queued.message.message_id
    ));
    assert!(activity.facts[0].scope().cursor < activity.facts[1].scope().cursor);
}

#[test]
fn only_an_unreleased_queued_prompt_can_be_canceled_once_by_receipt() {
    let (ledger, conversation_id) = ledger();
    let send = prompt(
        &ledger,
        &conversation_id,
        "send",
        AgentChatPromptDisposition::Send,
    );
    let released = prompt(
        &ledger,
        &conversation_id,
        "released",
        AgentChatPromptDisposition::Queue,
    );
    let queued = prompt(
        &ledger,
        &conversation_id,
        "queued",
        AgentChatPromptDisposition::Queue,
    );
    let cancel = |receipt: &str, conversation: &AgentChatConversationId, message: &str| {
        ledger.cancel_queued_agent_chat_prompt(
            &ReceiptId(receipt.into()),
            HostEpoch(1),
            conversation,
            message,
        )
    };
    let not_cancelable = |result: Result<gent_types::Receipt, LedgerError>| {
        matches!(
            result,
            Err(LedgerError::Rejected(
                gent_types::AgentChatRejection::QueuedPromptNotCancelable
            ))
        )
    };
    assert!(not_cancelable(cancel(
        "cancel-send",
        &conversation_id,
        &send.message.message_id
    )));
    assert!(not_cancelable(cancel(
        "cancel-foreign",
        &AgentChatConversationId("conversation-2".into()),
        &queued.message.message_id
    )));
    let claimed = ledger
        .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.message, send.message);
    ledger
        .fail_agent_chat_prompt_prelaunch(&send.message.message_id, "daemon-a", HostEpoch(1), "x")
        .unwrap();
    assert_eq!(
        ledger
            .claim_agent_chat_prompt_dispatch("daemon-a", HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .unwrap()
            .message,
        released.message
    );
    assert!(not_cancelable(cancel(
        "cancel-released",
        &conversation_id,
        &released.message.message_id
    )));
    let canceled = cancel(
        "cancel-queued",
        &conversation_id,
        &queued.message.message_id,
    )
    .unwrap();
    assert_eq!(
        cancel(
            "cancel-queued",
            &conversation_id,
            &queued.message.message_id
        )
        .unwrap(),
        canceled
    );
    assert!(
        cancel(
            "cancel-queued",
            &conversation_id,
            &released.message.message_id
        )
        .is_err()
    );
    assert!(not_cancelable(cancel(
        "cancel-again",
        &conversation_id,
        &queued.message.message_id
    )));
    let turns = ledger.list_run_turns(&queued.run_id.0).unwrap();
    assert_eq!(
        turns
            .iter()
            .find(|turn| turn.turn_id == queued.message.turn_id)
            .unwrap()
            .phase,
        gent_types::DurableTurnPhase::Cancelled
    );
    let activity = ledger
        .read_conversation_activity_page(&conversation_id.0, &queued.run_id.0, 0, 20)
        .unwrap();
    let canceled_facts = activity
        .facts
        .iter()
        .filter(|fact| {
            matches!(
                fact,
                gent_types::ConversationActivityFact::PromptCanceled { message_id, .. }
                    if message_id == &queued.message.message_id
            )
        })
        .count();
    assert_eq!(canceled_facts, 1);
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
    let decision_id = AgentChatDecisionId("0".into());
    ledger
        .save_pending_permission(&PermissionDecisionRequest {
            binding: PermissionDecisionBinding {
                decision_id: decision_id.clone(),
                request_idempotency_key: "codex:run-1:0".into(),
                conversation_id: conversation_id.clone(),
                run_id: send.run_id.clone(),
                turn_id: send.message.turn_id.clone(),
                policy_id: "policy-1".into(),
                policy_revision: 1,
                host_epoch: HostEpoch(1),
                request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
            },
            request: PermissionRequest {
                tool_name: "Command".into(),
                category: PermissionCategory::Command,
                input: None,
                child_id: None,
            },
        })
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
    assert_eq!(
        ledger
            .find_turn(&send.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        gent_types::DurableTurnPhase::Failed
    );
    assert!(
        ledger
            .pending_permission(&conversation_id, &send.run_id)
            .unwrap()
            .is_none()
    );
    assert!(
        ledger
            .read_conversation_activity_page(&conversation_id.0, &send.run_id.0, 0, 16)
            .unwrap()
            .facts
            .iter()
            .any(|fact| matches!(fact, gent_types::ConversationActivityFact::DecisionSettled { decision_id: settled, .. } if settled == &decision_id.0))
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
    assert_eq!(
        ledger
            .find_turn(&send.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        gent_types::DurableTurnPhase::Failed
    );
}

#[path = "support/agent_chat_prompt_dispatch_readiness_tests.rs"]
mod readiness_tests;
