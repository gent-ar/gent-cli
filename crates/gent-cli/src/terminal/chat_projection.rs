//! Pure selected-conversation projection for a future reconnectable terminal stream.

use gent_types::{
    HostEpoch, NormalizedTranscriptEvent, NormalizedTranscriptKind, NormalizedTranscriptPage,
};

/// A selected conversation's durable transcript view and cursor boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChatProjection {
    conversation_id: String,
    host_epoch: HostEpoch,
    events: Vec<NormalizedTranscriptEvent>,
    cursor: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum ProjectionError {
    #[error("transcript page belongs to another conversation")]
    WrongConversation,
    #[error("transcript cursor is not strictly ascending")]
    NonMonotonicCursor,
    #[error("transcript delta belongs to another selected conversation")]
    WrongEpoch,
}

impl ChatProjection {
    /// Builds display state from one durable page after connect or a cursor restart.
    pub(crate) fn from_page(
        conversation_id: String,
        host_epoch: HostEpoch,
        after_cursor: u64,
        page: NormalizedTranscriptPage,
    ) -> Result<Self, ProjectionError> {
        if page.conversation_id != conversation_id {
            return Err(ProjectionError::WrongConversation);
        }
        let mut projection = Self {
            conversation_id,
            host_epoch,
            events: Vec::new(),
            cursor: after_cursor,
        };
        for event in page.events {
            projection.apply(host_epoch, event)?;
        }
        Ok(projection)
    }

    /// Applies exactly one ordered daemon delta. An epoch change invalidates it.
    pub(crate) fn apply(
        &mut self,
        host_epoch: HostEpoch,
        event: NormalizedTranscriptEvent,
    ) -> Result<(), ProjectionError> {
        if host_epoch != self.host_epoch {
            return Err(ProjectionError::WrongEpoch);
        }
        if event.cursor <= self.cursor {
            return Err(ProjectionError::NonMonotonicCursor);
        }
        if event.kind == NormalizedTranscriptKind::AssistantMessage && !event.is_partial {
            self.replace_partial_tail(&event);
        }
        self.cursor = event.cursor;
        self.events.push(event);
        Ok(())
    }

    #[must_use]
    pub(crate) fn cursor(&self) -> u64 {
        self.cursor
    }

    #[must_use]
    pub(crate) fn events(&self) -> &[NormalizedTranscriptEvent] {
        &self.events
    }

    fn replace_partial_tail(&mut self, final_event: &NormalizedTranscriptEvent) {
        while self.events.last().is_some_and(|event| {
            event.kind == NormalizedTranscriptKind::AssistantMessage
                && event.is_partial
                && event.run_id == final_event.run_id
                && event.turn_id == final_event.turn_id
        }) {
            self.events.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{NormalizedTranscriptKind, NormalizedTranscriptPage};

    use super::{ChatProjection, HostEpoch, ProjectionError};

    fn event(cursor: u64, text: &str, is_partial: bool) -> gent_types::NormalizedTranscriptEvent {
        gent_types::NormalizedTranscriptEvent {
            cursor,
            event_id: format!("event-{cursor}"),
            turn_id: "turn".into(),
            run_id: "run".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: text.into(),
            is_partial,
        }
    }

    #[test]
    fn page_builds_state_and_deltas_are_strictly_cursor_and_epoch_bound() {
        let page = NormalizedTranscriptPage {
            conversation_id: "conversation".into(),
            events: vec![event(2, "normalized", false)],
            next_after_cursor: Some(2),
        };
        let mut view =
            ChatProjection::from_page("conversation".into(), HostEpoch(4), 1, page).unwrap();
        view.apply(HostEpoch(4), event(3, "normalized", false))
            .unwrap();
        assert_eq!(view.cursor(), 3);
        assert_eq!(view.events().len(), 2);
        assert_eq!(
            view.apply(HostEpoch(4), event(3, "normalized", false)),
            Err(ProjectionError::NonMonotonicCursor)
        );
        assert_eq!(
            view.apply(HostEpoch(5), event(4, "normalized", false)),
            Err(ProjectionError::WrongEpoch)
        );
    }

    #[test]
    fn final_assistant_output_replaces_only_its_partial_tail() {
        let page = NormalizedTranscriptPage {
            conversation_id: "conversation".into(),
            events: vec![event(1, "hel", true), event(2, "lo", true)],
            next_after_cursor: None,
        };
        let mut view =
            ChatProjection::from_page("conversation".into(), HostEpoch(1), 0, page).unwrap();
        view.apply(HostEpoch(1), event(3, "hello", false)).unwrap();
        assert_eq!(view.events().len(), 1);
        assert_eq!(view.events()[0].text, "hello");
        assert!(!view.events()[0].is_partial);
    }
}
