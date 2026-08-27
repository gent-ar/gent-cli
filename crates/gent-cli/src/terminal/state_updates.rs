use super::{UiRequestResult, UiState};

impl UiState {
    pub(crate) fn apply_request(&mut self, result: UiRequestResult) {
        if let Some(awaiting_turn) = result.awaiting_turn {
            self.awaiting_turn = awaiting_turn;
        }
        if let Some(session) = result.session {
            if let Some(index) = self
                .sessions
                .iter()
                .position(|item| item.session_id == session.session_id)
            {
                self.sessions[index] = session;
            }
        }
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
        if !was_selected {
            self.view = None;
        }
        self.clear_documents();
        if result.parent_run_id.is_some() || !was_selected {
            self.parent_run_id = result.parent_run_id;
        }
        if let Some(mode) = result.permission_mode {
            if let Some(conversation_id) = self.selected().map(|item| item.conversation_id.clone())
                && let Some(metadata) = self.metadata.get_mut(&conversation_id)
            {
                metadata.permission_mode = mode;
            }
        }
        self.notice = Some(result.notice);
    }

    pub(crate) fn clear_sent_prompt(&mut self) {
        self.input.clear();
        self.attachments.clear();
    }
}
