//! Pure terminal state for conversation selection and authority-gated prompt entry.

use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, ConversationListItem,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UiCommand {
    SelectNext,
    SelectPrevious,
    Quit,
    Insert(char),
    DeleteInput,
    SubmitPrompt,
    CreateConversation,
    CycleProvider,
    CycleEffort,
    CycleMode,
}

/// One protocol-neutral action emitted by the pure terminal reducer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum UiRequest {
    Create {
        selection: AgentChatSelection,
    },
    Send {
        conversation_id: String,
        text: String,
    },
}

/// The terminal reducer result; only the outer composition edge performs IPC.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum UiEffect {
    Continue,
    Quit,
    Request(UiRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UiState {
    conversations: Vec<ConversationListItem>,
    selected: Option<usize>,
    chat_enabled: bool,
    input: String,
    selection: AgentChatSelection,
    notice: Option<String>,
}

impl UiState {
    #[must_use]
    pub(crate) fn new(conversations: Vec<ConversationListItem>) -> Self {
        let selected = (!conversations.is_empty()).then_some(0);
        Self {
            conversations,
            selected,
            chat_enabled: false,
            input: String::new(),
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claude,
                model: "haiku".into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Ask,
            },
            notice: None,
        }
    }

    #[must_use]
    pub(crate) fn with_chat_input(mut self, chat_enabled: bool) -> Self {
        self.chat_enabled = chat_enabled;
        self
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

    #[must_use]
    pub(crate) const fn chat_enabled(&self) -> bool {
        self.chat_enabled
    }

    #[must_use]
    pub(crate) fn input(&self) -> &str {
        &self.input
    }

    #[must_use]
    pub(crate) fn selection(&self) -> &AgentChatSelection {
        &self.selection
    }

    #[must_use]
    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub(crate) fn replace_conversation(&mut self, item: ConversationListItem) {
        let index = self
            .conversations
            .iter()
            .position(|current| current.conversation_id == item.conversation_id)
            .unwrap_or_else(|| {
                self.conversations.insert(0, item);
                0
            });
        self.selected = Some(index);
        self.notice =
            Some("Durable request accepted; no provider is connected to this profile.".into());
    }

    pub(crate) fn set_notice(&mut self, value: String) {
        self.notice = Some(value);
    }

    /// Reduces a terminal action without performing I/O.
    pub(crate) fn apply(&mut self, command: UiCommand) -> UiEffect {
        match command {
            UiCommand::Quit => UiEffect::Quit,
            UiCommand::SelectNext => {
                self.select(|index, count| (index + 1).min(count.saturating_sub(1)));
                UiEffect::Continue
            }
            UiCommand::SelectPrevious => {
                self.select(|index, _| index.saturating_sub(1));
                UiEffect::Continue
            }
            UiCommand::Insert(value) if self.chat_enabled => {
                self.input.push(value);
                UiEffect::Continue
            }
            UiCommand::DeleteInput if self.chat_enabled => {
                self.input.pop();
                UiEffect::Continue
            }
            UiCommand::SubmitPrompt if self.chat_enabled => self.submit(),
            UiCommand::CreateConversation if self.chat_enabled => {
                UiEffect::Request(UiRequest::Create {
                    selection: self.selection.clone(),
                })
            }
            UiCommand::CycleProvider if self.chat_enabled => {
                self.selection.provider = match self.selection.provider {
                    AgentChatProvider::Claude => AgentChatProvider::Codex,
                    AgentChatProvider::Codex => AgentChatProvider::Claurst,
                    AgentChatProvider::Claurst => AgentChatProvider::Claude,
                };
                UiEffect::Continue
            }
            UiCommand::CycleEffort if self.chat_enabled => {
                self.selection.effort = match self.selection.effort {
                    AgentChatEffort::Low => AgentChatEffort::Medium,
                    AgentChatEffort::Medium => AgentChatEffort::High,
                    AgentChatEffort::High => AgentChatEffort::Low,
                };
                UiEffect::Continue
            }
            UiCommand::CycleMode if self.chat_enabled => {
                self.selection.mode = match self.selection.mode {
                    AgentChatMode::Ask => AgentChatMode::Plan,
                    AgentChatMode::Plan => AgentChatMode::Agent,
                    AgentChatMode::Agent => AgentChatMode::Ask,
                };
                UiEffect::Continue
            }
            _ => UiEffect::Continue,
        }
    }

    fn submit(&mut self) -> UiEffect {
        let Some(conversation_id) = self.selected().map(|value| value.conversation_id.clone())
        else {
            self.notice = Some("Create a conversation first with Ctrl+N.".into());
            return UiEffect::Continue;
        };
        let text = self.input.trim().to_owned();
        if text.is_empty() {
            return UiEffect::Continue;
        }
        self.input.clear();
        UiEffect::Request(UiRequest::Send {
            conversation_id,
            text,
        })
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
}
