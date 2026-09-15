use gent_protocol::LocalModelInstallState;
use gent_types::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatProvider, AgentChatRunId,
    DurableTurnPhase, HostEpoch, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    PermissionCategory, PermissionDecisionBinding, PermissionDecisionRequest, PermissionRequest,
    PermissionRequestDigest, TurnTerminalCause,
};

use super::{Output, TurnView};
use crate::cli_error::Failure;

fn event(
    cursor: u64,
    kind: NormalizedTranscriptKind,
    text: &str,
    partial: bool,
) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("event-{cursor}"),
        turn_id: "turn-1".into(),
        run_id: "run-1".into(),
        kind,
        text: text.into(),
        is_partial: partial,
        origin: None,
        attachments: Vec::new(),
    }
}

fn screen(outputs: Vec<Output>) -> String {
    outputs
        .into_iter()
        .map(|output| match output {
            Output::Reply(text) => text,
            Output::Status(text) => text
                .lines()
                .map(|line| format!("[stderr] {line}\n"))
                .collect(),
        })
        .collect()
}

fn view() -> TurnView {
    TurnView::new("gent", "conversation-1", "run-1")
}

#[test]
fn streamed_deltas_render_as_one_growing_reply_and_the_final_text_is_not_repeated() {
    let mut view = view();
    view.event(&event(
        1,
        NormalizedTranscriptKind::UserMessage,
        "say pineapple",
        false,
    ));
    for (cursor, delta) in [(2, "P"), (3, "INE"), (4, "APPLE")] {
        view.event(&event(
            cursor,
            NormalizedTranscriptKind::AssistantMessage,
            delta,
            true,
        ));
    }
    view.event(&event(
        5,
        NormalizedTranscriptKind::AssistantMessage,
        "PINEAPPLE",
        false,
    ));
    assert!(view.finish(DurableTurnPhase::Completed, None).is_ok());
    assert_eq!(screen(view.take()), "PINEAPPLE\n");
}

#[test]
fn a_final_reply_without_deltas_prints_once() {
    let mut view = view();
    view.event(&event(
        1,
        NormalizedTranscriptKind::AssistantMessage,
        "KIWI",
        false,
    ));
    assert_eq!(screen(view.take()), "KIWI\n");
}

#[test]
fn tools_and_notices_go_to_stderr_between_reply_segments() {
    let mut view = view();
    view.event(&event(
        1,
        NormalizedTranscriptKind::AssistantMessage,
        "Let me look",
        true,
    ));
    view.event(&event(
        2,
        NormalizedTranscriptKind::ToolActivity,
        "Read README.md\nline two",
        false,
    ));
    view.event(&event(
        3,
        NormalizedTranscriptKind::Thinking,
        "hidden",
        false,
    ));
    view.event(&event(
        4,
        NormalizedTranscriptKind::AssistantMessage,
        "Done.",
        false,
    ));
    assert_eq!(
        screen(view.take()),
        "Let me look\n[stderr]   · Read README.md…\nDone.\n"
    );
}

#[test]
fn a_waiting_permission_shows_the_request_and_ready_to_run_answers_once() {
    let request = PermissionDecisionRequest {
        binding: PermissionDecisionBinding {
            decision_id: AgentChatDecisionId("decision-7".into()),
            request_idempotency_key: "key".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
            turn_id: "turn-1".into(),
            policy_id: "policy".into(),
            policy_revision: 1,
            host_epoch: HostEpoch(1),
            request_digest_sha256: PermissionRequestDigest("digest".into()),
        },
        request: PermissionRequest::new(
            "Bash".into(),
            PermissionCategory::Command,
            Some(serde_json::json!({"command": "echo gentcheck > probe.txt"})),
            None,
        ),
    };
    let mut view = view();
    view.permission(&request);
    view.permission(&request);
    assert_eq!(
        screen(view.take()),
        "[stderr] ! Waiting for your permission to use Bash (Command)\n\
         [stderr]     echo gentcheck > probe.txt\n\
         [stderr]   Approve: gent permissions respond --conversation-id conversation-1 --run-id run-1 --decision-id decision-7 --decision approve-once\n\
         [stderr]   Deny:    gent permissions respond --conversation-id conversation-1 --run-id run-1 --decision-id decision-7 --decision deny\n"
    );
}

#[test]
fn model_download_progress_is_reported_in_steps_when_not_live() {
    let mut view = view();
    let gib = 1024 * 1024 * 1024;
    for downloaded in [0, gib / 20, gib / 8, gib / 2, gib] {
        view.download(
            "qwen3-1-7b-q4-k-m",
            &LocalModelInstallState::Downloading {
                downloaded_bytes: downloaded,
                total_bytes: gib,
            },
        );
    }
    assert_eq!(
        screen(view.take()),
        "[stderr] Downloading qwen3-1-7b-q4-k-m · 0% (0 B / 1.0 GiB)\n\
         [stderr] Downloading qwen3-1-7b-q4-k-m · 12% (128.0 MiB / 1.0 GiB)\n\
         [stderr] Downloading qwen3-1-7b-q4-k-m · 50% (512.0 MiB / 1.0 GiB)\n\
         [stderr] Downloading qwen3-1-7b-q4-k-m · 100% (1.0 GiB / 1.0 GiB)\n"
    );
}

#[test]
fn a_held_provider_install_explains_how_to_review_it() {
    let mut view = view();
    view.provider_install(AgentChatProvider::Claude);
    view.provider_install(AgentChatProvider::Claude);
    assert_eq!(
        screen(view.take()),
        "[stderr] ! Waiting for Claude to be installed before this prompt can run.\n\
         [stderr]   Review the install: gent provider readiness --conversation-id conversation-1 --run-id run-1\n"
    );
}

#[test]
fn failed_and_interrupted_turns_map_to_distinct_exit_codes_with_the_reason() {
    let mut view = view();
    view.event(&event(
        1,
        NormalizedTranscriptKind::Notice,
        "Claurst could not start: missing model",
        false,
    ));
    let failed = view.finish(DurableTurnPhase::Failed, None).unwrap_err();
    assert_eq!(failed.failure(), Failure::TurnFailed);
    assert_eq!(
        failed.to_string(),
        "the turn failed: Claurst could not start: missing model"
    );
    let interrupted = view
        .finish(DurableTurnPhase::Interrupted, None)
        .unwrap_err();
    assert_eq!(interrupted.failure(), Failure::TurnInterrupted);
    assert_eq!(
        view.finish(DurableTurnPhase::Cancelled, None)
            .unwrap_err()
            .failure(),
        Failure::TurnInterrupted
    );
    assert!(
        view.finish(
            DurableTurnPhase::Interrupted,
            Some(TurnTerminalCause::Steered)
        )
        .is_ok()
    );
}

#[test]
fn a_finished_plan_turn_prints_the_plan_and_the_commands_to_act_on_it() {
    let mut view = view();
    view.event(&event(
        1,
        NormalizedTranscriptKind::Plan,
        "1. Read\n2. Fix",
        false,
    ));
    assert!(view.finish(DurableTurnPhase::Completed, None).is_ok());
    assert_eq!(
        screen(view.take()),
        "1. Read\n2. Fix\n\
         [stderr] Plan ready.\n\
         [stderr]   Approve: gent plan start --conversation-id conversation-1\n\
         [stderr]   Reject:  gent plan reject --conversation-id conversation-1\n"
    );
}

#[test]
fn a_reviewed_provider_install_hold_is_announced_once_with_its_consent_command() {
    let mut view = view();
    let hold = crate::prompt_hold::tests::claude_install_hold();
    view.install_hold(&hold);
    view.install_hold(&hold);
    let screen = screen(view.take());
    assert_eq!(
        screen.matches("waiting for you to install Claude").count(),
        1,
        "{screen}"
    );
    assert!(
        screen.contains("[stderr]   Install: gent provider provision --conversation-id conversation-1 --run-id run-1 --prompt-receipt-id receipt-2 --reviewed-plan-digest "),
        "{screen}"
    );
}

fn user_prompt() -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        event_id: "user:message-7".into(),
        ..event(
            1,
            NormalizedTranscriptKind::UserMessage,
            "Which code?",
            false,
        )
    }
}

#[test]
fn a_turn_whose_provider_session_is_gone_prints_the_exact_continue_command() {
    let mut view = view();
    view.event(&user_prompt());
    view.event(&event(
        2,
        NormalizedTranscriptKind::Notice,
        gent_types::PROVIDER_SESSION_UNAVAILABLE_NOTICE,
        false,
    ));

    let failed = view
        .finish(
            DurableTurnPhase::Failed,
            Some(TurnTerminalCause::ProviderSessionUnavailable),
        )
        .unwrap_err();

    assert_eq!(failed.failure(), Failure::TurnFailed);
    assert!(screen(view.take()).contains(
        "[stderr] Continue from Gent's saved history: gent chat continue-from-history --conversation-id conversation-1 --message-id message-7\n"
    ));
}

#[test]
fn an_ordinary_failed_turn_never_offers_to_continue_from_saved_history() {
    let mut view = view();
    view.event(&user_prompt());

    assert!(view.finish(DurableTurnPhase::Failed, None).is_err());
    assert!(!screen(view.take()).contains("continue-from-history"));
}
