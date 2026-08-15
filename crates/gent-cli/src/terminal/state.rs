//! Pure selection state for the observer-safe conversation browser.

use gent_types::ConversationListItem;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiCommand {
    SelectNext,
    SelectPrevious,
    Quit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UiState {
    conversations: Vec<ConversationListItem>,
    selected: Option<usize>,
}

impl UiState {
    #[must_use]
    pub(crate) fn new(conversations: Vec<ConversationListItem>) -> Self {
        let selected = (!conversations.is_empty()).then_some(0);
        Self {
            conversations,
            selected,
        }
    }

    #[must_use]
    pub(crate) fn conversations(&self) -> &[ConversationListItem] {
        &self.conversations
    }

    #[must_use]
    pub(crate) fn selected(&self) -> Option<&ConversationListItem> {
        self.selected
            .and_then(|index| self.conversations.get(index))
    }

    /// Reduces a terminal action without performing I/O.
    pub(crate) fn apply(&mut self, command: UiCommand) -> bool {
        match command {
            UiCommand::Quit => true,
            UiCommand::SelectNext => {
                self.select(|index, count| (index + 1).min(count.saturating_sub(1)));
                false
            }
            UiCommand::SelectPrevious => {
                self.select(|index, _| index.saturating_sub(1));
                false
            }
        }
    }

    fn select(&mut self, next: impl FnOnce(usize, usize) -> usize) {
        if let Some(current) = self.selected {
            self.selected = Some(next(current, self.conversations.len()));
        }
    }
}

#[cfg(test)]
mod tests {
    use gent_types::ConversationListItem;

    use super::{UiCommand, UiState};

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
        assert!(!empty.apply(UiCommand::SelectNext));
    }

    #[test]
    fn quit_is_the_only_terminal_action() {
        let mut state = UiState::new(vec![item("one")]);
        assert!(!state.apply(UiCommand::SelectNext));
        assert!(state.apply(UiCommand::Quit));
    }
}
