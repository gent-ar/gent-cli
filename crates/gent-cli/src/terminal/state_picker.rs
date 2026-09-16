use gent_types::{AgentChatEffort, AgentChatMode};

use super::{SelectionPicker, UiState};
use crate::terminal::selection::{default_model, efforts, models, providers};

impl UiState {
    pub(super) fn open_picker(&mut self, picker: SelectionPicker) {
        if options(self, picker).is_empty() {
            self.notice = Some("Gentd has not listed any choices for this yet.".into());
            return;
        }
        self.selection_picker_index = current(self, picker);
        self.selection_picker = Some(picker);
        self.notice = Some(match picker {
            SelectionPicker::Permission => {
                "↑↓ choose · Enter apply · Bypass requires this explicit confirmation · Esc cancel"
            }
            _ => "↑↓ choose · Enter apply · Esc cancel",
        }
        .into());
    }

    pub(super) fn picker_move(&mut self, forward: bool) {
        let Some(picker) = self.selection_picker else {
            return;
        };
        let count = options(self, picker).len();
        self.selection_picker_index = if forward {
            (self.selection_picker_index + 1) % count
        } else {
            (self.selection_picker_index + count - 1) % count
        };
    }

    pub(super) fn apply_picker(&mut self) -> Option<super::UiEffect> {
        let picker = self.selection_picker.take()?;
        if picker == SelectionPicker::Permission {
            return Some(permission_request(self, self.selection_picker_index));
        }
        match (picker, self.selection_picker_index) {
            (SelectionPicker::Provider, index) => {
                self.selection.provider = providers(self.model_catalog.as_ref())[index].0;
            }
            (SelectionPicker::Model, index) => {
                options(self, picker)[index].clone_into(&mut self.selection.model);
            }
            (SelectionPicker::Effort, index) => {
                self.selection.effort =
                    efforts(self.model_catalog.as_ref(), &self.selection)[index];
            }
            (SelectionPicker::Mode, 0) => self.selection.mode = AgentChatMode::Ask,
            (SelectionPicker::Mode, 1) => self.selection.mode = AgentChatMode::Plan,
            (SelectionPicker::Mode, _) => self.selection.mode = AgentChatMode::Agent,
            (SelectionPicker::Permission, _) => unreachable!(),
        }
        if picker == SelectionPicker::Provider
            && let Some(model) = default_model(self.model_catalog.as_ref(), self.selection.provider)
        {
            self.selection.model = model;
        }
        if matches!(picker, SelectionPicker::Provider | SelectionPicker::Model) {
            super::selection_commands::fit_effort(self);
        }
        let Ok(effect) = super::super::state_switch::request(
            self.selected().map(|item| item.conversation_id.clone()),
            self.parent_run_id.clone(),
            self.selection.clone(),
            self.context_policy,
        ) else {
            self.new_conversation_selection = Some(self.selection.clone());
            self.notice = Some("Selection is ready for the next new conversation.".into());
            return None;
        };
        Some(effect)
    }

    pub(super) fn close_picker(&mut self) -> bool {
        self.selection_picker.take().is_some()
    }

    pub(crate) fn picker_line(&self) -> Option<String> {
        self.picker_view().map(|(title, values, selected)| {
            format!(
                "{title}: [{}] {}/{} · ↑↓ choose · Enter apply · Esc cancel",
                values[selected],
                selected + 1,
                values.len(),
            )
        })
    }

    pub(crate) fn picker_view(&self) -> Option<(String, Vec<String>, usize)> {
        self.selection_picker.map(|picker| {
            (
                title(picker).into(),
                options(self, picker),
                self.selection_picker_index,
            )
        })
    }
}

fn options(state: &UiState, picker: SelectionPicker) -> Vec<String> {
    match picker {
        SelectionPicker::Provider => providers(state.model_catalog.as_ref())
            .into_iter()
            .map(|(_, label)| label)
            .collect(),
        SelectionPicker::Model => model_options(state),
        SelectionPicker::Effort => effort_options(state),
        SelectionPicker::Mode => names(["Ask", "Plan", "Agent"]),
        SelectionPicker::Permission => names([
            "Ask every time",
            "Auto-approve edits",
            "Autonomous",
            "Bypass all permissions",
        ]),
    }
}

pub(super) fn effort_options(state: &UiState) -> Vec<String> {
    efforts(state.model_catalog.as_ref(), &state.selection)
        .into_iter()
        .map(|value| effort_name(value).to_owned())
        .collect()
}

fn names<const N: usize>(values: [&str; N]) -> Vec<String> {
    values.into_iter().map(str::to_owned).collect()
}

pub(super) fn model_options(state: &UiState) -> Vec<String> {
    models(state.model_catalog.as_ref(), state.selection.provider)
}

fn current(state: &UiState, picker: SelectionPicker) -> usize {
    options(state, picker)
        .iter()
        .position(|value| match picker {
            SelectionPicker::Provider => providers(state.model_catalog.as_ref())
                .iter()
                .any(|(provider, label)| *provider == state.selection.provider && label == value),
            SelectionPicker::Model => value == &state.selection.model,
            SelectionPicker::Effort => {
                value.eq_ignore_ascii_case(effort_name(state.selection.effort))
            }
            SelectionPicker::Mode => {
                value.eq_ignore_ascii_case(&format!("{:?}", state.selection.mode))
            }
            SelectionPicker::Permission => {
                value.eq_ignore_ascii_case(permission_name(state.permission_mode()))
            }
        })
        .unwrap_or(0)
}

fn effort_name(value: AgentChatEffort) -> &'static str {
    match value {
        AgentChatEffort::Low => "Low",
        AgentChatEffort::Medium => "Medium",
        AgentChatEffort::High => "High",
        AgentChatEffort::XHigh => "XHigh",
        AgentChatEffort::Max => "Max",
        AgentChatEffort::Ultra => "Ultra",
    }
}

fn title(picker: SelectionPicker) -> &'static str {
    match picker {
        SelectionPicker::Provider => "Provider",
        SelectionPicker::Model => "Model",
        SelectionPicker::Effort => "Effort",
        SelectionPicker::Mode => "Mode",
        SelectionPicker::Permission => "Permissions",
    }
}

fn permission_request(state: &mut UiState, index: usize) -> super::UiEffect {
    let mode = match index {
        0 => gent_types::PermissionMode::AskEveryTime,
        1 => gent_types::PermissionMode::AutoAcceptEdits,
        2 => gent_types::PermissionMode::Autonomous,
        _ => gent_types::PermissionMode::Bypass,
    };
    let Some(conversation_id) = state.selected().map(|item| item.conversation_id.clone()) else {
        state.notice = Some("Select a conversation before changing permissions.".into());
        return super::UiEffect::Continue;
    };
    let Some(workspace_id) = state.selected_workspace_id().map(str::to_owned) else {
        state.notice =
            Some("Workspace details are unavailable; refresh the conversation first.".into());
        return super::UiEffect::Continue;
    };
    super::UiEffect::Request(super::UiRequest::SetPermissionMode {
        conversation_id,
        workspace_id,
        mode,
        bypass_consent: mode == gent_types::PermissionMode::Bypass,
    })
}

fn permission_name(mode: gent_types::PermissionMode) -> &'static str {
    match mode {
        gent_types::PermissionMode::AskEveryTime => "Ask every time",
        gent_types::PermissionMode::AutoAcceptEdits => "Auto-approve edits",
        gent_types::PermissionMode::Autonomous => "Autonomous",
        gent_types::PermissionMode::Bypass => "Bypass all permissions",
    }
}
