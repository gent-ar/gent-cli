use super::{UiEffect, UiRequest, UiState};

pub(super) fn open(state: &mut UiState) -> UiEffect {
    if state.selected_automations().is_empty() {
        state.notice = Some("No Gent automations are configured for this workspace.".into());
        return UiEffect::Continue;
    }
    state.input.clear();
    state.automation_cursor = 0;
    state.automations_visible = true;
    UiEffect::Continue
}

impl UiState {
    pub(crate) fn automation_move(&mut self, next: bool) {
        let count = self.selected_automations().len();
        if self.automations_visible && count > 0 {
            self.automation_cursor = if next {
                (self.automation_cursor + 1).min(count - 1)
            } else {
                self.automation_cursor.saturating_sub(1)
            };
        }
    }

    pub(crate) fn automation_submit(&mut self) -> Option<UiEffect> {
        if !self.automations_visible {
            return None;
        }
        self.automations_visible = false;
        let Some(automation) = self
            .selected_automations()
            .get(self.automation_cursor)
            .cloned()
        else {
            return Some(UiEffect::Continue);
        };
        if !automation.enabled {
            self.notice = Some(format!("{} is disabled.", automation.name));
            return Some(UiEffect::Continue);
        }
        let Some(conversation_id) = self.selected().map(|item| item.conversation_id.clone()) else {
            self.notice = Some("Select a conversation before running an automation.".into());
            return Some(UiEffect::Continue);
        };
        Some(UiEffect::Request(UiRequest::RunAutomation {
            automation_id: automation.automation_id.0,
            conversation_id,
        }))
    }
}
