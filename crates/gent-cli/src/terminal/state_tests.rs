use gent_types::{ConversationListItem, ConversationStatus};

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
