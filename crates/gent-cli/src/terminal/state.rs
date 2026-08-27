use super::{ConversationView, state_switch::request};
use gent_types::{AgentChatSelection, AgentChatSession, ContextPolicy, ConversationListItem};
use std::{collections::BTreeMap, path::PathBuf};
#[path = "state_automations.rs"]
mod state_automations;
#[path = "state_constructor.rs"]
mod state_constructor;
#[path = "state_documents.rs"]
mod state_documents;
#[path = "state_picker.rs"]
mod state_picker;
#[path = "state_requests.rs"]
mod state_requests;
#[path = "state_search.rs"]
mod state_search;
#[path = "state_select.rs"]
mod state_select;
#[path = "state_sessions.rs"]
mod state_sessions;
#[path = "state_templates.rs"]
mod state_templates;
#[path = "state_thinking_commands.rs"]
mod state_thinking_commands;
#[path = "state_updates.rs"]
mod state_updates;
pub(crate) use state_requests::{UiEffect, UiRequest, UiRequestResult};
#[path = "ui_command.rs"]
mod ui_command;
pub(crate) use ui_command::UiCommand;
#[path = "selection_picker.rs"]
mod selection_picker;
pub(crate) use selection_picker::SelectionPicker;
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UiState {
    conversations: Vec<ConversationListItem>,
    selected: Option<usize>,
    chat_enabled: bool,
    input: String,
    pub(super) scroll_offset: u16,
    pub(super) attachments: Vec<PathBuf>,
    pub(super) metadata: BTreeMap<String, super::ConversationMetadata>,
    pub(super) sessions: Vec<AgentChatSession>,
    session_focus: bool,
    selected_session: Option<usize>,
    selection: AgentChatSelection,
    context_policy: ContextPolicy,
    pub(super) parent_run_id: Option<String>,
    pub(super) view: Option<ConversationView>,
    pub(super) notice: Option<String>,
    help_visible: bool,
    pub(super) activity_visible: bool,
    pub(super) documents: Vec<gent_protocol::WorkspaceDocumentRecord>,
    pub(super) documents_visible: bool,
    pub(super) document_cursor: usize,
    pub(super) templates: Vec<gent_types::PromptTemplateRecord>,
    pub(super) templates_visible: bool,
    pub(super) template_cursor: usize,
    pub(super) automations_visible: bool,
    pub(super) automation_cursor: usize,
    selection_picker: Option<SelectionPicker>,
    selection_picker_index: usize,
    pub(super) local_model_ids: Vec<String>,
    show_thinking: bool,
    awaiting_turn: bool,
    conversation_filter: String,
}
impl UiState {
    #[must_use]
    pub(crate) fn conversations(&self) -> &[ConversationListItem] {
        &self.conversations
    }
    #[must_use]
    pub(crate) fn selected(&self) -> Option<&ConversationListItem> {
        self.selected
            .and_then(|index| self.conversations.get(index))
    }
    pub(super) fn selected_index(&self) -> Option<usize> {
        self.selected
    }
    pub(super) fn selected_session_index(&self) -> Option<usize> {
        self.selected_session
    }
    pub(super) fn select_conversation(&mut self, conversation_id: &str) -> bool {
        let Some(index) = self
            .conversations
            .iter()
            .position(|item| item.conversation_id == conversation_id)
        else {
            return false;
        };
        self.selected = Some(index);
        self.view = None;
        self.parent_run_id = None;
        self.scroll_offset = 0;
        self.clear_documents();
        true
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
    pub(super) fn set_selection(&mut self, selection: AgentChatSelection) {
        self.selection = selection;
    }
    #[must_use]
    pub(crate) const fn context_policy(&self) -> ContextPolicy {
        self.context_policy
    }
    #[must_use]
    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
    pub(crate) const fn help_visible(&self) -> bool {
        self.help_visible
    }
    pub(crate) fn set_notice(&mut self, value: String) {
        self.notice = Some(value);
    }
    pub(crate) fn replace_input(&mut self, value: String) {
        self.input = value;
    }
    #[must_use]
    pub(crate) const fn show_thinking(&self) -> bool {
        self.show_thinking
    }
    #[must_use]
    pub(crate) const fn awaiting_turn(&self) -> bool {
        self.awaiting_turn
    }
    pub(super) fn finish_awaiting_turn(&mut self) {
        self.awaiting_turn = false;
    }
    pub(super) fn update_selected_run_count(&mut self, conversation_id: &str, run_count: u32) {
        if let Some(item) = self
            .selected
            .and_then(|index| self.conversations.get_mut(index))
            .filter(|item| item.conversation_id == conversation_id)
        {
            item.run_count = run_count;
        }
    }
    pub(crate) fn apply(&mut self, command: UiCommand) -> UiEffect {
        match command {
            UiCommand::Quit if self.close_picker() => UiEffect::Continue,
            UiCommand::Quit
                if self.help_visible
                    || self.activity_visible
                    || self.documents_visible
                    || self.templates_visible
                    || self.automations_visible =>
            {
                self.help_visible = false;
                self.activity_visible = false;
                self.documents_visible = false;
                self.templates_visible = false;
                self.automations_visible = false;
                UiEffect::Continue
            }
            UiCommand::Quit => UiEffect::Quit,
            UiCommand::ToggleHelp => {
                self.help_visible = !self.help_visible;
                UiEffect::Continue
            }
            UiCommand::ToggleActivity => {
                self.activity_visible = !self.activity_visible;
                UiEffect::Continue
            }
            UiCommand::ToggleThinking => {
                self.show_thinking = !self.show_thinking;
                self.notice = Some(
                    if self.show_thinking {
                        "Provider-emitted thinking is visible."
                    } else {
                        "Provider-emitted thinking is summarized."
                    }
                    .into(),
                );
                UiEffect::Continue
            }
            UiCommand::Interrupt if self.chat_enabled => match (
                self.selected().map(|item| item.conversation_id.clone()),
                self.parent_run_id.clone(),
            ) {
                (Some(conversation_id), Some(run_id)) => UiEffect::Request(UiRequest::Interrupt {
                    conversation_id,
                    run_id,
                }),
                _ => {
                    self.notice = Some("No active run is available to cancel.".into());
                    UiEffect::Continue
                }
            },
            UiCommand::SelectNext => {
                if self.selection_picker.is_some() {
                    self.picker_move(true);
                    UiEffect::Continue
                } else if self.documents_visible {
                    self.document_move(true);
                    UiEffect::Continue
                } else if self.templates_visible {
                    self.template_move(true);
                    UiEffect::Continue
                } else if self.automations_visible {
                    self.automation_move(true);
                    UiEffect::Continue
                } else if self.session_focus {
                    self.select_session(|index, count| (index + 1).min(count.saturating_sub(1)))
                } else {
                    self.select(|index, count| (index + 1).min(count.saturating_sub(1)))
                }
            }
            UiCommand::SelectPrevious => {
                if self.selection_picker.is_some() {
                    self.picker_move(false);
                    UiEffect::Continue
                } else if self.documents_visible {
                    self.document_move(false);
                    UiEffect::Continue
                } else if self.templates_visible {
                    self.template_move(false);
                    UiEffect::Continue
                } else if self.automations_visible {
                    self.automation_move(false);
                    UiEffect::Continue
                } else if self.session_focus {
                    self.select_session(|index, _| index.saturating_sub(1))
                } else {
                    self.select(|index, _| index.saturating_sub(1))
                }
            }
            UiCommand::FocusSessions => self.toggle_session_focus(),
            UiCommand::SubmitPrompt if self.selection_picker.is_some() => {
                self.apply_picker().unwrap_or(UiEffect::Continue)
            }
            UiCommand::SubmitPrompt if self.session_focus => self.open_session(),
            UiCommand::ScrollOlder => {
                self.scroll_offset = self.scroll_offset.saturating_add(8);
                UiEffect::Continue
            }
            UiCommand::ScrollNewer => {
                self.scroll_offset = self.scroll_offset.saturating_sub(8);
                UiEffect::Continue
            }
            UiCommand::Insert('?') if self.chat_enabled && self.input.is_empty() => {
                self.apply(UiCommand::ToggleHelp)
            }
            UiCommand::Insert(value) if self.chat_enabled => {
                self.input.push(value);
                UiEffect::Continue
            }
            UiCommand::InsertNewline if self.chat_enabled => {
                self.input.push('\n');
                UiEffect::Continue
            }
            UiCommand::Paste(value) if self.chat_enabled => state_submit::paste(self, value),
            UiCommand::DeleteInput if self.chat_enabled => {
                self.input.pop();
                UiEffect::Continue
            }
            UiCommand::SubmitPrompt if self.chat_enabled => {
                if self.document_submit() || self.template_submit() {
                    UiEffect::Continue
                } else if let Some(effect) = self.automation_submit() {
                    effect
                } else {
                    state_submit::submit(self)
                }
            }
            UiCommand::CreateConversation if self.chat_enabled => {
                UiEffect::Request(UiRequest::Create {
                    selection: self.selection.clone(),
                    session_id: self.focused_session_id(),
                })
            }
            UiCommand::CycleProvider if self.chat_enabled => {
                self.open_picker(SelectionPicker::Provider);
                UiEffect::Continue
            }
            UiCommand::CycleModel if self.chat_enabled => {
                self.open_picker(SelectionPicker::Model);
                UiEffect::Continue
            }
            UiCommand::CycleEffort if self.chat_enabled => {
                self.open_picker(SelectionPicker::Effort);
                UiEffect::Continue
            }
            UiCommand::CycleMode if self.chat_enabled => {
                self.open_picker(SelectionPicker::Mode);
                UiEffect::Continue
            }
            UiCommand::CyclePermission if self.chat_enabled => {
                self.open_picker(SelectionPicker::Permission);
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
}
#[path = "state_submit_notices.rs"]
mod notices;
#[path = "state_permissions.rs"]
mod permissions;
#[path = "state_submit_search.rs"]
mod search;
#[path = "state_selection_commands.rs"]
mod selection_commands;
#[path = "state_submit.rs"]
mod state_submit;
#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
