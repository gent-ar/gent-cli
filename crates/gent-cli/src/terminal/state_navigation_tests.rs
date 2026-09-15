use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, ConversationListItem, HostEpoch,
};

use super::{TranscriptScroll, UiCommand, UiEffect, UiRequest, UiState};
use crate::terminal::{ConversationView, input::command};

fn state() -> UiState {
    UiState::new(vec![ConversationListItem {
        conversation_id: "one".into(),
        run_count: 1,
    }])
    .with_chat_input(true)
    .with_command_catalog(Some(crate::terminal::commands::tests::catalog()))
}

fn queued(cursor: u64, message_id: &str) -> ConversationActivityFact {
    ConversationActivityFact::PromptQueued {
        scope: ConversationActivityScope {
            conversation_id: "one".into(),
            run_id: "run-1".into(),
            turn_id: format!("turn-{message_id}"),
            host_epoch: HostEpoch(1),
            cursor,
        },
        message_id: message_id.into(),
    }
}

#[test]
fn escape_clears_the_composer_and_never_quits() {
    let mut state = state();
    assert_eq!(
        command(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), true),
        Some(UiCommand::Dismiss)
    );
    state.apply(UiCommand::Insert('h'));
    state.apply(UiCommand::Insert('i'));
    assert_eq!(state.apply(UiCommand::Dismiss), UiEffect::Continue);
    assert_eq!(state.input(), "");
    assert_eq!(state.apply(UiCommand::Dismiss), UiEffect::Continue);
    assert_eq!(state.notice(), Some("Press Ctrl+Q to quit Gent."));
    assert_eq!(
        command(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
            true
        ),
        Some(UiCommand::Quit)
    );
    assert_eq!(state.apply(UiCommand::Quit), UiEffect::Quit);
}

#[test]
fn escape_closes_an_open_overlay_before_touching_the_draft() {
    let mut state = state();
    state.apply(UiCommand::Insert('x'));
    state.apply(UiCommand::ToggleHelp);
    assert!(state.help_visible());
    state.apply(UiCommand::Dismiss);
    assert!(!state.help_visible());
    assert_eq!(state.input(), "x");
}

#[test]
fn scrolling_back_pins_the_view_until_the_user_returns_to_the_end() {
    let mut state = state();
    assert_eq!(state.transcript_top(40), 40);
    state.apply(UiCommand::ScrollOlder);
    assert_eq!(state.scroll(), TranscriptScroll::Pinned(32));
    assert_eq!(state.transcript_top(90), 32);
    state.apply(UiCommand::FollowLatest);
    assert_eq!(state.transcript_top(90), 90);
    assert_eq!(
        command(KeyEvent::new(KeyCode::End, KeyModifiers::NONE), true),
        Some(UiCommand::FollowLatest)
    );
}

#[test]
fn steer_sends_every_queued_prompt_oldest_first_and_cancel_removes_the_newest() {
    let mut state = state().with_view(Some(
        ConversationView::new("one", None, None)
            .with_activity(Some(vec![queued(9, "second"), queued(4, "first")])),
    ));
    assert_eq!(
        command(
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            true
        ),
        Some(UiCommand::SteerQueued)
    );
    assert_eq!(
        state.apply(UiCommand::SteerQueued),
        UiEffect::Request(UiRequest::SteerQueued {
            conversation_id: "one".into(),
            message_ids: vec!["first".into(), "second".into()],
        })
    );
    assert_eq!(
        command(
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            true
        ),
        Some(UiCommand::CancelQueued)
    );
    assert_eq!(
        state.apply(UiCommand::CancelQueued),
        UiEffect::Request(UiRequest::CancelQueued {
            conversation_id: "one".into(),
            message_id: "second".into(),
        })
    );
    for text in ["/steer", "/cancel-queued"] {
        text.chars().for_each(|value| {
            state.apply(UiCommand::Insert(value));
        });
        assert!(matches!(
            state.apply(UiCommand::SubmitPrompt),
            UiEffect::Request(_)
        ));
        assert_eq!(state.input(), "");
    }
}

#[test]
fn queue_keys_explain_when_nothing_is_queued() {
    let mut state = state();
    assert_eq!(state.apply(UiCommand::SteerQueued), UiEffect::Continue);
    assert!(
        state
            .notice()
            .is_some_and(|notice| notice.contains("No prompts are queued"))
    );
    assert_eq!(state.apply(UiCommand::CancelQueued), UiEffect::Continue);
}

#[test]
fn enter_while_a_turn_is_running_queues_the_prompt_instead_of_sending_it() {
    let mut state = state().with_status(Some(gent_types::ConversationStatus {
        conversation_id: "one".into(),
        runs: vec![gent_types::ConversationRunStatus {
            run_id: "run-1".into(),
            parent_run_id: None,
            provider: "claurst".into(),
            active_turn_id: Some("turn-1".into()),
            live_status: None,
        }],
    }));
    "and then this".chars().for_each(|value| {
        state.apply(UiCommand::Insert(value));
    });
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Queue {
            conversation_id: "one".into(),
            text: "and then this".into(),
            attachments: Vec::new(),
        })
    );
}

fn failed_turn(cause: Option<gent_types::TurnTerminalCause>) -> ConversationView {
    let prompt = gent_types::NormalizedTranscriptEvent {
        cursor: 1,
        event_id: "user:message-7".into(),
        turn_id: "turn-7".into(),
        run_id: "run-1".into(),
        kind: gent_types::NormalizedTranscriptKind::UserMessage,
        text: "Which code did I give you?".into(),
        is_partial: false,
        origin: None,
        attachments: Vec::new(),
    };
    ConversationView::new(
        "one",
        None,
        Some(gent_types::NormalizedTranscriptPage {
            conversation_id: "one".into(),
            events: vec![prompt],
            next_after_cursor: None,
        }),
    )
    .with_activity(Some(vec![ConversationActivityFact::Terminal {
        scope: ConversationActivityScope {
            conversation_id: "one".into(),
            run_id: "run-1".into(),
            turn_id: "turn-7".into(),
            host_epoch: HostEpoch(1),
            cursor: 3,
        },
        phase: gent_types::TurnPhase::Failed,
        cause,
    }]))
}

fn submit(state: &mut UiState, text: &str) -> UiEffect {
    text.chars().for_each(|value| {
        state.apply(UiCommand::Insert(value));
    });
    state.apply(UiCommand::SubmitPrompt)
}

#[test]
fn slash_continue_resends_the_turn_whose_provider_session_was_lost() {
    let mut state = state().with_view(Some(failed_turn(Some(
        gent_types::TurnTerminalCause::ProviderSessionUnavailable,
    ))));
    assert_eq!(
        submit(&mut state, "/continue"),
        UiEffect::Request(UiRequest::ContinueFromHistory {
            conversation_id: "one".into(),
            message_id: "message-7".into(),
        })
    );
    assert_eq!(state.input(), "");
}

#[test]
fn slash_continue_is_refused_for_an_ordinary_failed_turn() {
    let mut state = state().with_view(Some(failed_turn(None)));
    assert_eq!(submit(&mut state, "/continue"), UiEffect::Continue);
    assert_eq!(
        state.notice(),
        Some("No turn here lost its provider session.")
    );
}
