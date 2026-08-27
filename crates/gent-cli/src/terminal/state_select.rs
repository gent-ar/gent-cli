use super::{UiEffect, UiState};

impl UiState {
    pub(super) fn select(&mut self, next: impl FnOnce(usize, usize) -> usize) -> UiEffect {
        let visible = self.visible_conversation_indices();
        if let Some(current) = self.selected
            && let Some(position) = visible.iter().position(|index| *index == current)
        {
            let next_position = next(position, visible.len());
            let selected = visible[next_position];
            if selected != current {
                self.selected = Some(selected);
                self.view = None;
                self.parent_run_id = None;
                self.scroll_offset = 0;
                self.clear_documents();
                return UiEffect::Refresh(self.conversations[selected].conversation_id.clone());
            }
        }
        UiEffect::Continue
    }
}
