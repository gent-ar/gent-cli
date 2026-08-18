//! Pure terminal state for conversation selection and authority-gated prompt entry.

use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, ContextPolicy,
    ConversationListItem, ConversationStatus,
};

use super::{
    selection::{default_model, next_model},
    state_switch::request,
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
    CycleModel,
    CycleEffort,
    CycleMode,
    CycleContext,
    SwitchSelection,
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
    Switch {
        conversation_id: String,
        parent_run_id: String,
        selection: AgentChatSelection,
        context_policy: ContextPolicy,
    },
}

/// Result returned by the terminal composition edge after one durable request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UiRequestResult {
    pub(crate) conversation: ConversationListItem,
    pub(crate) parent_run_id: Option<String>,
    pub(crate) notice: String,
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
    context_policy: ContextPolicy,
    parent_run_id: Option<String>,
    status: Option<ConversationStatus>,
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
            context_policy: ContextPolicy::Preserve,
            parent_run_id: None,
            status: None,
            notice: None,
        }
    }

    #[must_use]
    pub(crate) fn with_chat_input(mut self, chat_enabled: bool) -> Self {
        self.chat_enabled = chat_enabled;
        self
    }

    /// Adds one content-free status only while it belongs to the selected conversation.
    #[must_use]
    pub(crate) fn with_status(mut self, status: Option<ConversationStatus>) -> Self {
        self.status = status.filter(|status| {
            self.selected()
                .is_some_and(|item| item.conversation_id == status.conversation_id)
        });
        self.parent_run_id = self
            .status
            .as_ref()
            .and_then(|status| match status.runs.as_slice() {
                [run] if !run.run_id.is_empty() => Some(run.run_id.clone()),
                _ => None,
            });
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

    /// Returns only status data that is known to match the current selection.
    #[must_use]
    pub(crate) fn selected_status(&self) -> Option<&ConversationStatus> {
        self.status.as_ref().filter(|status| {
            self.selected()
                .is_some_and(|item| item.conversation_id == status.conversation_id)
        })
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
    pub(crate) const fn context_policy(&self) -> ContextPolicy {
        self.context_policy
    }

    #[must_use]
    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub(crate) fn apply_request(&mut self, result: UiRequestResult) {
        let item = result.conversation;
        let was_selected = self
            .selected()
            .is_some_and(|current| current.conversation_id == item.conversation_id);
        let index = self
            .conversations
            .iter()
            .position(|current| current.conversation_id == item.conversation_id)
            .unwrap_or_else(|| {
                self.conversations.insert(0, item);
                0
            });
        self.selected = Some(index);
        self.status = None;
        if result.parent_run_id.is_some() || !was_selected {
            self.parent_run_id = result.parent_run_id;
        }
        self.notice = Some(result.notice);
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
                self.selection.model = default_model(self.selection.provider).into();
                UiEffect::Continue
            }
            UiCommand::CycleModel if self.chat_enabled => {
                self.selection.model = next_model(&self.selection).into();
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
            UiCommand::CycleContext if self.chat_enabled => {
                self.context_policy = match self.context_policy {
                    ContextPolicy::Preserve => ContextPolicy::Clear,
                    ContextPolicy::Clear => ContextPolicy::Preserve,
                };
                UiEffect::Continue
            }
            UiCommand::SwitchSelection if self.chat_enabled => request(
                self.selected().map(|value| value.conversation_id.clone()),
                self.parent_run_id.clone(),
                self.selection.clone(),
                self.context_policy,
            )
            .unwrap_or_else(|notice| {
                self.notice = Some(notice.into());
                UiEffect::Continue
            }),
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
            let selected = next(current, self.conversations.len());
            if selected != current {
                self.selected = Some(selected);
                self.status = None;
                self.parent_run_id = None;
            }
        }
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
