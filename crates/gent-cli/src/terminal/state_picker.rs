use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider};

use super::{SelectionPicker, UiState};
use crate::terminal::selection::default_model;

impl UiState {
    pub(super) fn open_picker(&mut self, picker: SelectionPicker) {
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
        let Some(picker) = self.selection_picker.take() else {
            return None;
        };
        if picker == SelectionPicker::Permission {
            return Some(permission_request(self, self.selection_picker_index));
        };
        match (picker, self.selection_picker_index) {
            (SelectionPicker::Provider, 0) => self.selection.provider = AgentChatProvider::Claurst,
            (SelectionPicker::Provider, 1) => self.selection.provider = AgentChatProvider::Claude,
            (SelectionPicker::Provider, _) => self.selection.provider = AgentChatProvider::Codex,
            (SelectionPicker::Model, index) => {
                self.selection.model = options(self, picker)[index].clone()
            }
            (SelectionPicker::Effort, index) => {
                self.selection.effort = effort(options(self, picker)[index].as_str())
            }
            (SelectionPicker::Mode, 0) => self.selection.mode = AgentChatMode::Ask,
            (SelectionPicker::Mode, 1) => self.selection.mode = AgentChatMode::Plan,
            (SelectionPicker::Mode, _) => self.selection.mode = AgentChatMode::Agent,
            (SelectionPicker::Permission, _) => unreachable!(),
        }
        if picker == SelectionPicker::Provider {
            self.selection.model = model_options(self)
                .into_iter()
                .next()
                .unwrap_or_else(|| default_model(self.selection.provider).into());
        }
        match super::super::state_switch::request(
            self.selected().map(|item| item.conversation_id.clone()),
            self.parent_run_id.clone(),
            self.selection.clone(),
            self.context_policy,
        ) {
            Ok(effect) => Some(effect),
            Err(_) => {
                self.notice = Some("Selection is ready for the next new conversation.".into());
                None
            }
        }
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
        SelectionPicker::Provider => names(["Gent (Claurst)", "Claude", "Codex"]),
        SelectionPicker::Model => model_options(state),
        SelectionPicker::Effort => effort_options(state),
        SelectionPicker::Mode => names(["Ask", "Plan", "Agent"]),
        SelectionPicker::Permission => names([
            "Ask every action",
            "Read-only",
            "Auto-approve edits",
            "Autonomous",
            "Bypass all permissions",
        ]),
    }
}

pub(super) fn effort_options(state: &UiState) -> Vec<String> {
    match state.selection.provider {
        AgentChatProvider::Codex => names(["Low", "Medium", "High", "XHigh", "Max", "Ultra"]),
        AgentChatProvider::Claude | AgentChatProvider::Claurst => names(["Low", "Medium", "High"]),
    }
}

fn effort(value: &str) -> AgentChatEffort {
    match value {
        "Low" => AgentChatEffort::Low,
        "Medium" => AgentChatEffort::Medium,
        "High" => AgentChatEffort::High,
        "XHigh" => AgentChatEffort::XHigh,
        "Max" => AgentChatEffort::Max,
        "Ultra" => AgentChatEffort::Ultra,
        _ => AgentChatEffort::Medium,
    }
}

fn names<const N: usize>(values: [&str; N]) -> Vec<String> {
    values.into_iter().map(str::to_owned).collect()
}

pub(super) fn model_options(state: &UiState) -> Vec<String> {
    match state.selection.provider {
        AgentChatProvider::Claude => names(["haiku", "sonnet", "claude-fable-5", "opus"]),
        AgentChatProvider::Codex => names([
            "default",
            "gpt-5.6",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5.3-codex-spark",
        ]),
        AgentChatProvider::Claurst if !state.local_model_ids.is_empty() => {
            state.local_model_ids.clone()
        }
        AgentChatProvider::Claurst => vec![default_model(AgentChatProvider::Claurst).into()],
    }
}

fn current(state: &UiState, picker: SelectionPicker) -> usize {
    options(state, picker)
        .iter()
        .position(|value| match picker {
            SelectionPicker::Provider => {
                let provider = match state.selection.provider {
                    AgentChatProvider::Claude => "claude",
                    AgentChatProvider::Codex => "codex",
                    AgentChatProvider::Claurst => "claurst",
                };
                value.eq_ignore_ascii_case(provider)
                    || value.to_ascii_lowercase().contains(provider)
            }
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
        0 => gent_types::PermissionMode::Default,
        1 => gent_types::PermissionMode::Plan,
        2 => gent_types::PermissionMode::AutoAcceptEdits,
        3 => gent_types::PermissionMode::Autonomous,
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
        gent_types::PermissionMode::Default => "Ask every action",
        gent_types::PermissionMode::Plan => "Read-only",
        gent_types::PermissionMode::AutoAcceptEdits => "Auto-approve edits",
        gent_types::PermissionMode::Autonomous => "Autonomous",
        gent_types::PermissionMode::Bypass => "Bypass all permissions",
    }
}
