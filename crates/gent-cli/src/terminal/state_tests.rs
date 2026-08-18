use gent_types::{ContextPolicy, ConversationListItem, ConversationRunStatus, ConversationStatus};

use super::{UiCommand, UiEffect, UiRequest, UiState};

fn item(id: &str) -> ConversationListItem {
    ConversationListItem {
        conversation_id: id.into(),
        run_count: 1,
    }
}

#[test]
fn selection_is_clamped_and_empty_state_is_safe() {
    let mut state = UiState::new(vec![item("one"), item("two")]);
    state.apply(UiCommand::SelectPrevious);
    assert_eq!(state.selected().unwrap().conversation_id, "one");
    state.apply(UiCommand::SelectNext);
    state.apply(UiCommand::SelectNext);
    assert_eq!(state.selected().unwrap().conversation_id, "two");
    let mut empty = UiState::new(Vec::new());
    assert!(empty.selected().is_none());
    assert_eq!(empty.apply(UiCommand::SelectNext), UiEffect::Continue);
}

#[test]
fn selection_drops_status_that_belongs_to_the_previous_conversation() {
    let mut state =
        UiState::new(vec![item("one"), item("two")]).with_status(Some(ConversationStatus {
            conversation_id: "one".into(),
            runs: Vec::new(),
        }));
    assert!(state.selected_status().is_some());
    state.apply(UiCommand::SelectNext);
    assert!(state.selected_status().is_none());
}

#[test]
fn quit_is_the_only_terminal_action() {
    let mut state = UiState::new(vec![item("one")]);
    assert_eq!(state.apply(UiCommand::SelectNext), UiEffect::Continue);
    assert_eq!(state.apply(UiCommand::Quit), UiEffect::Quit);
}

#[test]
fn enabled_input_emits_a_typed_request_without_doing_io() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    state.apply(UiCommand::Insert('h'));
    state.apply(UiCommand::Insert('i'));
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Send {
            conversation_id: "one".into(),
            text: "hi".into(),
        })
    );
}

#[test]
fn slash_goal_is_bound_to_the_selected_conversation_and_exact_current_run() {
    let status = ConversationStatus {
        conversation_id: "one".into(),
        runs: vec![ConversationRunStatus {
            run_id: "run-one".into(),
            parent_run_id: None,
            provider: "codex".into(),
            active_turn_id: None,
            live_status: None,
        }],
    };
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_status(Some(status));
    for character in "/goal finish switching safely".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(
        state.apply(UiCommand::SubmitPrompt),
        UiEffect::Request(UiRequest::Goal {
            conversation_id: "one".into(),
            run_id: "run-one".into(),
            summary: "finish switching safely".into(),
        })
    );
}

#[test]
fn slash_goal_never_guesses_a_run_binding() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    for character in "/goal keep going".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(state.apply(UiCommand::SubmitPrompt), UiEffect::Continue);
    assert_eq!(
        state.notice(),
        Some("Run status is unavailable; refusing to guess a /goal binding.")
    );
    assert_eq!(state.input(), "/goal keep going");
}

#[test]
fn empty_slash_goal_is_not_sent_as_a_provider_prompt() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    for character in "/goal ".chars() {
        state.apply(UiCommand::Insert(character));
    }
    assert_eq!(state.apply(UiCommand::SubmitPrompt), UiEffect::Continue);
    assert_eq!(
        state.notice(),
        Some("`/goal` requires a concise summary; no provider work was started")
    );
}

#[test]
fn selection_switch_is_parent_bound_and_carries_each_control() {
    let status = ConversationStatus {
        conversation_id: "one".into(),
        runs: vec![ConversationRunStatus {
            run_id: "parent".into(),
            parent_run_id: None,
            provider: "codex".into(),
            active_turn_id: None,
            live_status: None,
        }],
    };
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_status(Some(status));
    state.apply(UiCommand::CycleProvider);
    state.apply(UiCommand::CycleModel);
    state.apply(UiCommand::CycleEffort);
    state.apply(UiCommand::CycleMode);
    state.apply(UiCommand::CycleContext);
    let UiEffect::Request(UiRequest::Switch {
        conversation_id,
        parent_run_id,
        selection,
        context_policy,
    }) = state.apply(UiCommand::SwitchSelection)
    else {
        panic!("expected parent-bound selection switch");
    };
    assert_eq!(conversation_id, "one");
    assert_eq!(parent_run_id, "parent");
    assert_eq!(selection.model, "gpt-5.6");
    assert_eq!(context_policy, ContextPolicy::Clear);
}

#[test]
fn selection_switch_refuses_to_guess_an_unknown_parent() {
    let mut state = UiState::new(vec![item("one")]).with_chat_input(true);
    assert_eq!(state.apply(UiCommand::SwitchSelection), UiEffect::Continue);
    assert_eq!(
        state.notice(),
        Some("Run status is unavailable; refusing to guess a switch parent.")
    );
}

#[test]
fn selection_switch_refuses_an_ambiguous_status_hierarchy() {
    let mut state = UiState::new(vec![item("one")])
        .with_chat_input(true)
        .with_status(Some(ConversationStatus {
            conversation_id: "one".into(),
            runs: vec![
                ConversationRunStatus {
                    run_id: "first".into(),
                    parent_run_id: None,
                    provider: "claude".into(),
                    active_turn_id: None,
                    live_status: None,
                },
                ConversationRunStatus {
                    run_id: "second".into(),
                    parent_run_id: Some("first".into()),
                    provider: "codex".into(),
                    active_turn_id: None,
                    live_status: None,
                },
            ],
        }));
    assert_eq!(state.apply(UiCommand::SwitchSelection), UiEffect::Continue);
    assert!(state.notice().unwrap().contains("refusing to guess"));
}
