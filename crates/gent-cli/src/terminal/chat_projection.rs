//! Pure selected-conversation projection for a future reconnectable terminal stream.

use gent_types::{HostEpoch, NormalizedTranscriptEvent, NormalizedTranscriptPage};

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
    #[error("transcript snapshot belongs to another conversation")]
    WrongConversation,
    #[error("transcript cursor is not strictly ascending")]
    NonMonotonicCursor,
    #[error("transcript delta belongs to another selected conversation")]
    WrongEpoch,
}

impl ChatProjection {
    /// Replaces all display state from a daemon snapshot after connect or resync.
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
}

#[cfg(test)]
mod tests {
    use gent_types::{NormalizedTranscriptKind, NormalizedTranscriptPage};

    use super::{ChatProjection, HostEpoch, ProjectionError};

    fn event(cursor: u64) -> gent_types::NormalizedTranscriptEvent {
        gent_types::NormalizedTranscriptEvent {
            cursor,
            event_id: format!("event-{cursor}"),
            turn_id: "turn".into(),
            run_id: "run".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "normalized".into(),
            is_partial: false,
        }
    }

    #[test]
    fn snapshot_replaces_state_and_deltas_are_strictly_cursor_and_epoch_bound() {
        let page = NormalizedTranscriptPage {
            conversation_id: "conversation".into(),
            events: vec![event(2)],
            next_after_cursor: Some(2),
        };
        let mut view =
            ChatProjection::from_page("conversation".into(), HostEpoch(4), 1, page).unwrap();
        view.apply(HostEpoch(4), event(3)).unwrap();
        assert_eq!(view.cursor(), 3);
        assert_eq!(view.events().len(), 2);
        assert_eq!(
            view.apply(HostEpoch(4), event(3)),
            Err(ProjectionError::NonMonotonicCursor)
        );
        assert_eq!(
            view.apply(HostEpoch(5), event(4)),
            Err(ProjectionError::WrongEpoch)
        );
    }
}
