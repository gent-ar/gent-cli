use super::{UiEffect, UiState};
use gent_types::PromptTemplateRecord;

pub(super) fn open(state: &mut UiState) -> UiEffect {
    state.input.clear();
    UiEffect::ListTemplates
}

impl UiState {
    pub(crate) fn set_templates(&mut self, templates: Vec<PromptTemplateRecord>) {
        self.templates = templates;
        self.template_cursor = 0;
        self.templates_visible = true;
    }
    pub(crate) fn template_move(&mut self, next: bool) {
        if self.templates_visible && !self.templates.is_empty() {
            let last = self.templates.len() - 1;
            self.template_cursor = if next {
                (self.template_cursor + 1).min(last)
            } else {
                self.template_cursor.saturating_sub(1)
            };
        }
    }
    pub(crate) fn template_submit(&mut self) -> bool {
        if !self.templates_visible {
            return false;
        }
        if let Some(template) = self.templates.get(self.template_cursor) {
            self.input = format!("/template {}", template.template_id);
            self.notice =
                Some("Template selected. Add name=value variables, then press Enter.".into());
        }
        self.templates_visible = false;
        true
    }
}
