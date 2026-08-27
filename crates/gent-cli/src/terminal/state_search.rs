use super::UiState;

impl UiState {
    pub(super) fn set_conversation_filter(&mut self, value: &str) -> Option<String> {
        self.conversation_filter = value.trim().to_lowercase();
        if let Some(index) = self.visible_conversation_indices().into_iter().next() {
            self.selected = Some(index);
            self.view = None;
            self.parent_run_id = None;
            self.scroll_offset = 0;
            self.clear_documents();
            return Some(self.conversations[index].conversation_id.clone());
        }
        None
    }

    pub(crate) fn visible_conversation_indices(&self) -> Vec<usize> {
        self.conversations
            .iter()
            .enumerate()
            .filter(|(_, conversation)| self.matches_conversation(&conversation.conversation_id))
            .map(|(index, _)| index)
            .collect()
    }

    pub(crate) fn conversation_filter(&self) -> Option<&str> {
        (!self.conversation_filter.is_empty()).then_some(&self.conversation_filter)
    }

    fn matches_conversation(&self, conversation_id: &str) -> bool {
        if self.conversation_filter.is_empty() {
            return true;
        }
        let mut text = conversation_id.to_lowercase();
        if let Some(metadata) = self.metadata.get(conversation_id) {
            if let Some(title) = &metadata.title {
                text.push_str(title);
            }
            if let Some(recap) = &metadata.recap {
                text.push_str(recap);
            }
        }
        text.to_lowercase().contains(&self.conversation_filter)
    }
}
