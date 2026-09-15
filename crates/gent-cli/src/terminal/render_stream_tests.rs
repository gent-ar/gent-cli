use gent_types::{
    ConversationListItem, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    NormalizedTranscriptPage,
};
use ratatui::{Terminal, backend::TestBackend};

use super::render;
use crate::terminal::{ConversationView, UiState, state::UiCommand};

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

fn view(events: Vec<NormalizedTranscriptEvent>) -> ConversationView {
    ConversationView::new(
        "one",
        None,
        Some(NormalizedTranscriptPage {
            conversation_id: "one".into(),
            events,
            next_after_cursor: None,
        }),
    )
}

fn state(events: Vec<NormalizedTranscriptEvent>) -> UiState {
    UiState::new(vec![ConversationListItem {
        conversation_id: "one".into(),
        run_count: 1,
    }])
    .with_chat_input(true)
    .with_view(Some(view(events)))
}

fn screen(state: &UiState, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| render(frame, state)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|row| {
            (0..width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn streaming_deltas_render_as_one_live_message() {
    let state = state(vec![
        event(
            1,
            NormalizedTranscriptKind::UserMessage,
            "list fruits",
            false,
        ),
        event(2, NormalizedTranscriptKind::AssistantMessage, "1.", true),
        event(
            3,
            NormalizedTranscriptKind::AssistantMessage,
            " Fig\n2.",
            true,
        ),
        event(4, NormalizedTranscriptKind::AssistantMessage, " Kiwi", true),
    ]);
    assert_eq!(state.selected_transcript().len(), 2);
    let screen = screen(&state, 120, 40).join("\n");
    assert_eq!(screen.matches("Gent · streaming").count(), 1, "{screen}");
    assert!(screen.contains("1. Fig"), "{screen}");
    assert!(screen.contains("2. Kiwi"), "{screen}");
}

#[test]
fn the_final_message_replaces_the_live_stream() {
    let state = state(vec![
        event(1, NormalizedTranscriptKind::AssistantMessage, "PINE", true),
        event(2, NormalizedTranscriptKind::AssistantMessage, "APPLE", true),
        event(
            3,
            NormalizedTranscriptKind::AssistantMessage,
            "PINEAPPLE",
            false,
        ),
    ]);
    let screen = screen(&state, 120, 40).join("\n");
    assert!(!screen.contains("streaming"), "{screen}");
    assert_eq!(screen.matches("PINEAPPLE").count(), 1, "{screen}");
}

#[test]
fn the_chat_follows_new_messages_even_when_lines_wrap() {
    let long = "word ".repeat(40);
    let mut events = (0..30)
        .map(|index| {
            event(
                index,
                NormalizedTranscriptKind::AssistantMessage,
                &format!("{long}{index}"),
                false,
            )
        })
        .collect::<Vec<_>>();
    let mut state = state(events.clone());
    assert!(screen(&state, 100, 30).join("\n").contains("word 29"));
    state.apply(UiCommand::ScrollOlder);
    let pinned = screen(&state, 100, 30);
    assert!(pinned.join("\n").contains("scrolled back"));
    events.push(event(
        30,
        NormalizedTranscriptKind::AssistantMessage,
        "NEWEST",
        false,
    ));
    state.apply_view(view(events));
    let still_pinned = screen(&state, 100, 30);
    assert_eq!(pinned[8..20], still_pinned[8..20]);
    state.apply(UiCommand::FollowLatest);
    assert!(screen(&state, 100, 30).join("\n").contains("NEWEST"));
}

#[test]
fn a_plan_ready_for_review_shows_its_steps_and_the_review_actions() {
    let plan = gent_types::PlanArtifact {
        plan_id: gent_types::ReviewedPlanId("plan-1".into()),
        conversation_id: gent_types::AgentChatConversationId("one".into()),
        source_run_id: gent_types::AgentChatRunId("run-1".into()),
        source_turn_id: "turn-1".into(),
        revision: gent_types::PlanRevision(2),
        content_digest_sha256: "d".repeat(64),
        status: gent_types::PlanStatus::ReadyForReview,
        content: "1. Read the code\n2. Fix the bug".into(),
    };
    let fact = gent_types::ConversationActivityFact::PlanUpdated {
        scope: gent_types::ConversationActivityScope {
            conversation_id: "one".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            host_epoch: gent_types::HostEpoch(1),
            cursor: 3,
        },
        plan,
    };
    let state = UiState::new(vec![ConversationListItem {
        conversation_id: "one".into(),
        run_count: 1,
    }])
    .with_chat_input(true)
    .with_view(Some(view(Vec::new()).with_activity(Some(vec![fact]))));
    let screen = screen(&state, 220, 50).join("\n");
    assert!(
        screen.contains("Plan · revision 2 · ready for review"),
        "{screen}"
    );
    assert!(screen.contains("2. Fix the bug"), "{screen}");
    assert!(
        screen.contains("Approve: gent plan start --conversation-id one"),
        "{screen}"
    );
    assert!(
        screen.contains("Reject:  gent plan reject --conversation-id one"),
        "{screen}"
    );
}

#[test]
fn a_pending_permission_stays_visible_below_a_long_followed_transcript() {
    let long = (0..60)
        .map(|index| {
            event(
                index,
                NormalizedTranscriptKind::AssistantMessage,
                &format!("line {index}"),
                false,
            )
        })
        .collect::<Vec<_>>();
    let request = gent_types::PermissionDecisionRequest {
        binding: gent_types::PermissionDecisionBinding {
            decision_id: gent_types::AgentChatDecisionId("decision-1".into()),
            request_idempotency_key: "key".into(),
            conversation_id: gent_types::AgentChatConversationId("one".into()),
            run_id: gent_types::AgentChatRunId("run-1".into()),
            turn_id: "turn-1".into(),
            policy_id: "policy".into(),
            policy_revision: 1,
            host_epoch: gent_types::HostEpoch(1),
            request_digest_sha256: gent_types::PermissionRequestDigest("digest".into()),
        },
        request: gent_types::PermissionRequest::new(
            "Grep".into(),
            gent_types::PermissionCategory::Read,
            None,
            None,
        ),
    };
    let state = UiState::new(vec![ConversationListItem {
        conversation_id: "one".into(),
        run_count: 1,
    }])
    .with_chat_input(true)
    .with_view(Some(view(long).with_pending_permission(Some(request))));
    let screen = screen(&state, 120, 30).join("\n");
    assert!(screen.contains("Permission required · Grep"), "{screen}");
}

#[test]
fn a_held_provider_install_shows_the_review_and_the_commands_to_consent() {
    let state = UiState::new(vec![ConversationListItem {
        conversation_id: "one".into(),
        run_count: 1,
    }])
    .with_chat_input(true)
    .with_view(Some(view(Vec::new()).with_install_hold(Some(
        crate::prompt_hold::tests::claude_install_hold(),
    ))));
    let screen = screen(&state, 400, 30).join("\n");
    assert!(
        screen.contains("This prompt is waiting for you to install Claude"),
        "{screen}"
    );
    assert!(
        screen.contains("@anthropic-ai/claude-code@2.1.0"),
        "{screen}"
    );
    assert!(
        screen.contains(
            "Install: gent provider provision --conversation-id conversation-1 --run-id run-1 --prompt-receipt-id receipt-2"
        ),
        "{screen}"
    );
}
