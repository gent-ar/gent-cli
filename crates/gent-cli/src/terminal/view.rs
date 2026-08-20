//! Selected-conversation data loaded through Gentd's existing read protocol.

use gent_types::{ConversationStatus, NormalizedTranscriptEvent, NormalizedTranscriptPage};

/// A content-bearing view is always scoped to exactly one selected conversation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationView {
    conversation_id: String,
    status: Option<ConversationStatus>,
    transcript: Vec<NormalizedTranscriptEvent>,
}

impl ConversationView {
    #[must_use]
    pub(crate) fn new(
        conversation_id: String,
        status: Option<ConversationStatus>,
        transcript: Option<NormalizedTranscriptPage>,
    ) -> Self {
        let transcript = transcript
            .filter(|page| page.conversation_id == conversation_id)
            .map_or_else(Vec::new, |page| page.events);
        Self {
            conversation_id: conversation_id.clone(),
            status: status.filter(|value| value.conversation_id == conversation_id),
            transcript,
        }
    }

    #[must_use]
    pub(crate) fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    #[must_use]
    pub(crate) fn status(&self) -> Option<&ConversationStatus> {
        self.status.as_ref()
    }

    #[must_use]
    pub(crate) fn transcript(&self) -> &[NormalizedTranscriptEvent] {
        &self.transcript
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{
        NormalizedTranscriptEvent, NormalizedTranscriptKind, NormalizedTranscriptPage,
    };

    use super::ConversationView;

    #[test]
    fn view_rejects_data_for_another_conversation() {
        let view = ConversationView::new(
            "selected".into(),
            None,
            Some(NormalizedTranscriptPage {
                conversation_id: "other".into(),
                events: vec![NormalizedTranscriptEvent {
                    cursor: 1,
                    event_id: "event".into(),
                    turn_id: "turn".into(),
                    run_id: "run".into(),
                    kind: NormalizedTranscriptKind::AssistantMessage,
                    text: "must not render".into(),
                    is_partial: false,
                }],
                next_after_cursor: None,
            }),
        );
        assert_eq!(view.conversation_id(), "selected");
        assert!(view.transcript().is_empty());
    }
}
