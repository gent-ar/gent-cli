use super::{UiEffect, UiRequest, UiState};

pub(super) fn command(state: &mut UiState, argument: &str) -> UiEffect {
    if argument.is_empty() {
        state.input.clear();
        state.commands.picker = Some(state.selected_index().unwrap_or(0));
        return UiEffect::Continue;
    }
    let (candidate, prompt) = argument
        .split_once(char::is_whitespace)
        .map_or((argument, ""), |(candidate, prompt)| {
            (candidate, prompt.trim())
        });
    if !state.select_conversation(candidate) {
        state.notice = Some(format!(
            "No conversation {candidate} is listed; use /resume to choose one."
        ));
        return UiEffect::Continue;
    }
    state.input.clear();
    if prompt.is_empty() {
        UiEffect::Refresh(candidate.into())
    } else {
        UiEffect::Request(UiRequest::Send {
            conversation_id: candidate.into(),
            text: prompt.into(),
            attachments: Vec::new(),
        })
    }
}

impl UiState {
    pub(crate) const fn conversation_picker(&self) -> Option<usize> {
        self.commands.picker
    }

    pub(crate) fn command_catalog(
        &self,
    ) -> Option<&gent_protocol::agent_chat_commands::CommandCatalog> {
        self.view
            .as_ref()
            .and_then(super::super::ConversationView::commands)
            .or(self.commands.home.as_ref())
    }

    pub(crate) fn conversation_picker_move(&mut self, forward: bool) {
        let last = self.conversations().len().saturating_sub(1);
        self.commands.picker = self.commands.picker.map(|cursor| {
            if forward {
                (cursor + 1).min(last)
            } else {
                cursor.saturating_sub(1)
            }
        });
    }

    pub(crate) fn conversation_picker_submit(&mut self) -> Option<UiEffect> {
        let cursor = self.commands.picker.take()?;
        let conversation_id = self.conversations().get(cursor)?.conversation_id.clone();
        self.select_conversation(&conversation_id);
        Some(UiEffect::Refresh(conversation_id))
    }

    pub(crate) fn close_conversation_picker(&mut self) -> bool {
        self.commands.picker.take().is_some()
    }

    pub(crate) fn command_catalog_refresh(&self) -> UiEffect {
        let loading = self.command_catalog().is_some_and(|catalog| {
            catalog.listing == gent_protocol::agent_chat_commands::CommandListing::Loading
        });
        match self.selected() {
            Some(item) if loading && self.input == "/" => {
                UiEffect::Refresh(item.conversation_id.clone())
            }
            _ => UiEffect::Continue,
        }
    }
}
