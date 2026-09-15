use super::{UiEffect, UiState};

const SCROLL_STEP: u16 = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum TranscriptScroll {
    #[default]
    Follow,
    Pinned(u16),
}

impl UiState {
    pub(super) fn dismiss(&mut self) -> UiEffect {
        if self.close_picker() || self.close_conversation_picker() {
            return UiEffect::Continue;
        }
        if self.help_visible
            || self.activity_visible
            || self.documents_visible
            || self.templates_visible
            || self.automations_visible
        {
            self.help_visible = false;
            self.activity_visible = false;
            self.documents_visible = false;
            self.templates_visible = false;
            self.automations_visible = false;
        } else if self.session_focus {
            self.session_focus = false;
        } else if !self.input.is_empty() {
            self.input.clear();
            self.notice = Some("Message cleared.".into());
        } else {
            self.notice = Some("Press Ctrl+Q to quit Gent.".into());
        }
        UiEffect::Continue
    }

    pub(super) fn move_selection(&mut self, forward: bool) -> UiEffect {
        if self.selection_picker.is_some() {
            self.picker_move(forward);
        } else if self.commands.picker.is_some() {
            self.conversation_picker_move(forward);
        } else if self.documents_visible {
            self.document_move(forward);
        } else if self.templates_visible {
            self.template_move(forward);
        } else if self.automations_visible {
            self.automation_move(forward);
        } else if self.session_focus {
            return if forward {
                self.select_session(|index, count| (index + 1).min(count.saturating_sub(1)))
            } else {
                self.select_session(|index, _| index.saturating_sub(1))
            };
        } else {
            return if forward {
                self.select(|index, count| (index + 1).min(count.saturating_sub(1)))
            } else {
                self.select(|index, _| index.saturating_sub(1))
            };
        }
        UiEffect::Continue
    }

    pub(super) fn scroll_by(&mut self, newer: bool) -> UiEffect {
        let limit = self.scroll_limit.get();
        let top = match self.scroll {
            TranscriptScroll::Follow => limit,
            TranscriptScroll::Pinned(top) => top.min(limit),
        };
        self.scroll = if newer {
            let next = top.saturating_add(SCROLL_STEP);
            if next >= limit {
                TranscriptScroll::Follow
            } else {
                TranscriptScroll::Pinned(next)
            }
        } else {
            TranscriptScroll::Pinned(top.saturating_sub(SCROLL_STEP))
        };
        UiEffect::Continue
    }

    #[must_use]
    pub(crate) const fn scroll(&self) -> TranscriptScroll {
        self.scroll
    }

    pub(crate) fn transcript_top(&self, limit: u16) -> u16 {
        self.scroll_limit.set(limit);
        match self.scroll {
            TranscriptScroll::Follow => limit,
            TranscriptScroll::Pinned(top) => top.min(limit),
        }
    }
}
