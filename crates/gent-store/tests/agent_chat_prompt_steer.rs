use gent_ports::{
    AgentChatProjectionLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger,
    AgentChatQueuedPromptLedger, AgentChatWorkspaceLedger, ConversationActivityLedger,
    ConversationLedger, Ledger, LedgerError, TranscriptLedger,
};
use gent_store::SqliteLedger;
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatPromptCreate, AgentChatPromptDisposition, AgentChatPromptSaved, AgentChatProvider,
    AgentChatRejection, AgentChatRequestId, AgentChatRunId, AgentChatSelection,
    ConversationActivityFact, DurableTurnPhase, HostEpoch, NormalizedTranscriptAppend,
    NormalizedTranscriptKind, ReceiptId, WorkspaceRecord,
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

fn not_steerable(result: Result<impl std::fmt::Debug, LedgerError>) -> bool {
    matches!(
        result,
        Err(LedgerError::Rejected(
            AgentChatRejection::QueuedPromptNotSteerable
        ))
    )
}

fn activity_types(ledger: &SqliteLedger, prompt: &AgentChatPromptSaved) -> Vec<String> {
    ledger
        .read_conversation_activity_page(&prompt.message.conversation_id, &prompt.run_id.0, 0, 50)
        .unwrap()
        .facts
        .iter()
        .map(|fact| {
            serde_json::to_value(fact).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn a_steer_request_is_receipt_bound_to_one_waiting_queued_prompt() {
    let (ledger, conversation_id) = ledger();
    let send = active(&ledger, &conversation_id);
    let queued = prompt(
        &ledger,
        &conversation_id,
        "queued",
        AgentChatPromptDisposition::Queue,
    );

    assert!(not_steerable(steer(
        &ledger,
        &conversation_id,
        "steer-send",
        &send
    )));
    let (receipt, run_id) = steer(&ledger, &conversation_id, "steer-1", &queued).unwrap();
    assert_eq!(run_id, queued.run_id);
    assert_eq!(
        receipt.idempotency_key,
        format!(
            "agent-chat-steer-queued-prompt:{}",
            queued.message.message_id
        )
    );
    assert_eq!(
        steer(&ledger, &conversation_id, "steer-1", &queued)
            .unwrap()
            .0,
        receipt
    );
    assert!(not_steerable(steer(
        &ledger,
        &conversation_id,
        "steer-2",
        &queued
    )));
    assert_eq!(activity_types(&ledger, &queued), ["promptQueued"]);
}

#[test]
fn a_steered_prompt_leaves_the_queue_once_and_is_recorded_inside_the_running_turn() {
    let (ledger, conversation_id) = ledger();
    let send = active(&ledger, &conversation_id);
    let queued = prompt(
        &ledger,
        &conversation_id,
        "queued",
        AgentChatPromptDisposition::Queue,
    );
    steer(&ledger, &conversation_id, "steer-1", &queued).unwrap();
    let assistant = ledger
        .append_normalized_transcript(
            &conversation_id,
            &NormalizedTranscriptAppend {
                event_id: "assistant-before-steer".into(),
                turn_id: send.message.turn_id.clone(),
                run_id: send.run_id.0.clone(),
                kind: NormalizedTranscriptKind::AssistantMessage,
                text: "working".into(),
                is_partial: false,
            },
        )
        .unwrap();

    let (claimed, steer_receipt) = ledger
        .claim_steered_agent_chat_prompt(DAEMON, HostEpoch(1), &send.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.message, queued.message);
    assert_eq!(steer_receipt, ReceiptId("steer-1".into()));
    assert!(
        ledger
            .claim_steered_agent_chat_prompt(DAEMON, HostEpoch(1), &send.run_id)
            .unwrap()
            .is_none()
    );
    assert!(
        ledger
            .claim_agent_chat_prompt_dispatch(DAEMON, HostEpoch(1), AgentChatProvider::Codex)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        ledger.cancel_queued_agent_chat_prompt(
            &ReceiptId("cancel".into()),
            HostEpoch(1),
            &conversation_id,
            &queued.message.message_id
        ),
        Err(LedgerError::Rejected(
            AgentChatRejection::QueuedPromptNotCancelable
        ))
    ));

    let id = &queued.message.message_id;
    ledger
        .deliver_steered_agent_chat_prompt(id, DAEMON, HostEpoch(1), &send.message.turn_id)
        .unwrap();
    assert!(
        ledger
            .deliver_steered_agent_chat_prompt(id, DAEMON, HostEpoch(1), &send.message.turn_id)
            .is_err()
    );
    assert!(
        ledger
            .start_steered_agent_chat_prompt_turn(id, DAEMON, HostEpoch(1))
            .is_err()
    );

    assert_eq!(
        ledger
            .find_turn(&queued.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Completed
    );
    assert_eq!(
        ledger
            .find_turn(&send.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Active
    );
    assert_eq!(
        activity_types(&ledger, &queued),
        ["promptQueued", "promptSteered"]
    );
    let steered = ledger
        .read_conversation_activity_page(&conversation_id.0, &send.run_id.0, 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .find_map(|fact| match fact {
            ConversationActivityFact::PromptSteered {
                scope,
                message_id,
                receipt_id,
                transcript_cursor,
            } => Some((scope.turn_id, message_id, receipt_id, transcript_cursor)),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        steered,
        (
            send.message.turn_id.clone(),
            queued.message.message_id.clone(),
            "steer-1".into(),
            assistant.cursor
        )
    );

    for restored in [0, 1] {
        let page = ledger
            .agent_chat_projection_page(&conversation_id, restored, 100)
            .unwrap();
        let position = |predicate: &dyn Fn(&gent_types::AgentChatProjectionEvent) -> bool| {
            page.events
                .iter()
                .filter(|event| predicate(event))
                .map(|event| event.cursor)
                .collect::<Vec<_>>()
        };
        let steered = position(&|event| event.payload["activity"]["type"] == "promptSteered");
        let working = position(&|event| event.payload["eventId"] == assistant.event_id);
        assert_eq!(steered.len(), 1);
        assert_eq!(
            page.events
                .iter()
                .find(|event| event.cursor == steered[0])
                .unwrap()
                .payload["turnId"],
            send.message.turn_id
        );
        assert!(working[0] < steered[0]);
    }
}

#[test]
fn a_later_steer_waits_behind_an_unsteered_prompt_and_releases_in_queue_order() {
    let (ledger, conversation_id) = ledger();
    let send = active(&ledger, &conversation_id);
    let first = prompt(
        &ledger,
        &conversation_id,
        "first",
        AgentChatPromptDisposition::Queue,
    );
    let second = prompt(
        &ledger,
        &conversation_id,
        "second",
        AgentChatPromptDisposition::Queue,
    );
    steer(&ledger, &conversation_id, "steer-second", &second).unwrap();
    assert!(
        ledger
            .claim_steered_agent_chat_prompt(DAEMON, HostEpoch(1), &send.run_id)
            .unwrap()
            .is_none()
    );

    steer(&ledger, &conversation_id, "steer-first", &first).unwrap();
    let claimed = [0, 1].map(|_| {
        ledger
            .claim_steered_agent_chat_prompt(DAEMON, HostEpoch(1), &send.run_id)
            .unwrap()
            .unwrap()
            .0
            .message
            .message_id
    });
    assert_eq!(
        claimed,
        [
            first.message.message_id.clone(),
            second.message.message_id.clone()
        ]
    );
    for prompt in [&second, &first] {
        ledger
            .release_agent_chat_prompt_unstarted_launch(
                &prompt.message.message_id,
                DAEMON,
                HostEpoch(1),
            )
            .unwrap();
    }
    ledger
        .settle_agent_chat_prompt_terminal(
            &send.message.message_id,
            DAEMON,
            HostEpoch(1),
            DurableTurnPhase::Completed,
        )
        .unwrap();

    let next = ledger
        .claim_agent_chat_prompt_dispatch(DAEMON, HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    assert_eq!(next.message, first.message);
    assert_eq!(
        activity_types(&ledger, &first)
            .iter()
            .filter(|kind| *kind == "promptReleased")
            .count(),
        1
    );
    assert!(!activity_types(&ledger, &first).contains(&"promptSteered".to_owned()));
}

#[test]
fn a_steer_the_provider_runs_as_its_own_turn_is_released_and_settles_normally() {
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
    ledger
        .settle_agent_chat_prompt_terminal(
            &send.message.message_id,
            DAEMON,
            HostEpoch(1),
            DurableTurnPhase::Completed,
        )
        .unwrap();

    let id = &queued.message.message_id;
    ledger
        .start_steered_agent_chat_prompt_turn(id, DAEMON, HostEpoch(1))
        .unwrap();
    ledger
        .settle_agent_chat_prompt_terminal(id, DAEMON, HostEpoch(1), DurableTurnPhase::Completed)
        .unwrap();

    assert_eq!(
        activity_types(&ledger, &queued),
        ["promptQueued", "terminal", "promptReleased", "terminal"]
    );
    assert_eq!(
        ledger
            .find_turn(&queued.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Completed
    );
}

#[test]
fn a_steer_written_before_a_crash_is_never_replayed_by_the_successor() {
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
            .find_turn(&queued.message.turn_id)
            .unwrap()
            .unwrap()
            .phase,
        DurableTurnPhase::Failed
    );
}

fn terminal_cause(
    ledger: &SqliteLedger,
    prompt: &AgentChatPromptSaved,
) -> Option<Option<gent_types::TurnTerminalCause>> {
    ledger
        .read_conversation_activity_page(&prompt.message.conversation_id, &prompt.run_id.0, 0, 50)
        .unwrap()
        .facts
        .into_iter()
        .find_map(|fact| match fact {
            ConversationActivityFact::Terminal { scope, cause, .. }
                if scope.turn_id == prompt.message.turn_id =>
            {
                Some(cause)
            }
            _ => None,
        })
}

#[test]
fn an_interrupt_caused_by_a_steer_is_typed_apart_from_a_user_stop() {
    let (ledger, conversation_id) = ledger();
    let run_id = AgentChatRunId("run-1".into());
    assert!(
        !ledger
            .interrupt_active_turn_for_steer(HostEpoch(1), &conversation_id, &run_id)
            .unwrap()
    );

    let steered = active(&ledger, &conversation_id);
    assert!(
        ledger
            .interrupt_active_turn_for_steer(HostEpoch(1), &conversation_id, &run_id)
            .unwrap()
    );
    assert!(
        ledger
            .interrupt_active_turn_for_steer(HostEpoch(1), &conversation_id, &run_id)
            .unwrap()
    );
    ledger
        .settle_agent_chat_prompt_terminal(
            &steered.message.message_id,
            DAEMON,
            HostEpoch(1),
            DurableTurnPhase::Interrupted,
        )
        .unwrap();
    assert_eq!(
        terminal_cause(&ledger, &steered),
        Some(Some(gent_types::TurnTerminalCause::Steered))
    );
    let projected = ledger
        .agent_chat_projection_page(&conversation_id, 0, 100)
        .unwrap()
        .events
        .into_iter()
        .find(|event| event.payload["activity"]["type"] == "terminal")
        .unwrap();
    assert_eq!(projected.payload["activity"]["phase"], "interrupted");
    assert_eq!(projected.payload["activity"]["cause"], "steered");

    let stopped = prompt(
        &ledger,
        &conversation_id,
        "stopped",
        AgentChatPromptDisposition::Send,
    );
    ledger
        .claim_agent_chat_prompt_dispatch(DAEMON, HostEpoch(1), AgentChatProvider::Codex)
        .unwrap()
        .unwrap();
    let id = &stopped.message.message_id;
    ledger
        .begin_agent_chat_prompt_launch(id, DAEMON, HostEpoch(1))
        .unwrap();
    ledger
        .confirm_agent_chat_prompt_started(id, DAEMON, HostEpoch(1))
        .unwrap();
    ledger
        .settle_agent_chat_prompt_terminal(id, DAEMON, HostEpoch(1), DurableTurnPhase::Interrupted)
        .unwrap();
    assert_eq!(terminal_cause(&ledger, &stopped), Some(None));
}
