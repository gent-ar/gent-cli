use gent_types::{
    ConversationArtifact, ConversationArtifactKind, ConversationArtifactStatus,
    MAX_CONVERSATION_LABEL_BYTES, bounded_excerpt,
};
use sha2::{Digest, Sha256};

const LABEL_TITLE_PROVIDER: &str = "gent";
const LABEL_TITLE_MODEL_VERSION: &str = "conversation-label";

#[must_use]
pub fn label_title(
    conversation_id: &str,
    label: &str,
    source_turn_ids: Vec<String>,
    artifact_id: String,
) -> Option<ConversationArtifact> {
    let text = bounded_excerpt(label.trim(), MAX_CONVERSATION_LABEL_BYTES);
    if text.is_empty() || source_turn_ids.is_empty() || artifact_id.is_empty() {
        return None;
    }
    let input_digest = format!("{:x}", Sha256::digest(text.as_bytes()));
    Some(ConversationArtifact {
        artifact_id,
        conversation_id: conversation_id.into(),
        kind: ConversationArtifactKind::Title,
        source_turn_ids,
        provider: LABEL_TITLE_PROVIDER.into(),
        model_version: LABEL_TITLE_MODEL_VERSION.into(),
        input_digest,
        status: ConversationArtifactStatus::Completed,
        text: Some(text),
        supersedes_artifact_id: None,
    })
}

#[cfg(test)]
mod tests {
    use gent_types::{
        MAX_CONVERSATION_LABEL_BYTES, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    };

    use super::label_title;
    use crate::conversation_summary::{ConversationSummaryKind, scheduled_requests};

    fn turn() -> Vec<String> {
        vec!["turn-1".into()]
    }

    fn completed_turn() -> Vec<NormalizedTranscriptEvent> {
        vec![NormalizedTranscriptEvent {
            cursor: 1,
            event_id: "event-1".into(),
            turn_id: "turn-1".into(),
            run_id: "run-1".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "done".into(),
            is_partial: false,
            origin: None,
            attachments: Vec::new(),
        }]
    }

    #[test]
    fn a_blank_label_titles_nothing() {
        assert!(label_title("conversation", "   ", turn(), "label:1".into()).is_none());
        assert!(label_title("conversation", "Bounds audit", vec![], "label:1".into()).is_none());
    }

    #[test]
    fn a_label_title_is_trimmed_and_bounded_like_the_label_itself() {
        let artifact =
            label_title("conversation", "  Bounds audit  ", turn(), "label:1".into()).unwrap();
        assert_eq!(artifact.text.as_deref(), Some("Bounds audit"));
        let long = "x".repeat(MAX_CONVERSATION_LABEL_BYTES + 7);
        let clipped = label_title("conversation", &long, turn(), "label:2".into()).unwrap();
        assert_eq!(clipped.text.unwrap().len(), MAX_CONVERSATION_LABEL_BYTES);
    }

    #[test]
    fn a_label_title_keeps_the_summarizer_from_generating_another_title() {
        let events = completed_turn();
        let titled =
            vec![label_title("conversation", "Bounds audit", turn(), "label:1".into()).unwrap()];
        let untitled = scheduled_requests("conversation", "claude", "haiku", &events, &[]).unwrap();
        assert!(
            untitled
                .iter()
                .any(|request| request.kind == ConversationSummaryKind::Title)
        );
        let scheduled =
            scheduled_requests("conversation", "claude", "haiku", &events, &titled).unwrap();
        assert!(
            scheduled
                .iter()
                .all(|request| request.kind != ConversationSummaryKind::Title)
        );
    }
}
