//! Read-only selected conversation data owned by the terminal state.

use gent_types::{ConversationStatus, NormalizedTranscriptEvent};

use super::{ConversationView, UiState};

impl UiState {
    #[must_use]
    pub(crate) fn with_view(mut self, view: Option<ConversationView>) -> Self {
        self.view = view.filter(|view| {
            self.selected()
                .is_some_and(|item| item.conversation_id == view.conversation_id())
        });
        self.parent_run_id = self.view.as_ref().and_then(run_id);
        self
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_status(self, status: Option<ConversationStatus>) -> Self {
        let view = status.map(|status| {
            ConversationView::new(status.conversation_id.clone(), Some(status), None)
        });
        self.with_view(view)
    }

    #[must_use]
    pub(crate) fn selected_status(&self) -> Option<&ConversationStatus> {
        self.view
            .as_ref()
            .and_then(|view| view.status())
            .filter(|status| {
                self.selected()
                    .is_some_and(|item| item.conversation_id == status.conversation_id)
            })
    }

    #[must_use]
    pub(crate) fn selected_transcript(&self) -> &[NormalizedTranscriptEvent] {
        self.view
            .as_ref()
            .filter(|view| {
                self.selected()
                    .is_some_and(|item| item.conversation_id == view.conversation_id())
            })
            .map_or(&[], ConversationView::transcript)
    }

    pub(crate) fn apply_view(&mut self, view: ConversationView) {
        if self
            .selected()
            .is_some_and(|item| item.conversation_id == view.conversation_id())
        {
            self.parent_run_id = run_id(&view);
            self.view = Some(view);
        }
    }
}

fn run_id(view: &ConversationView) -> Option<String> {
    view.status()
        .and_then(|status| match status.runs.as_slice() {
            [run] if !run.run_id.is_empty() => Some(run.run_id.clone()),
            _ => None,
        })
}
