use gent_ports::{ConversationContentReader, LedgerError, TranscriptLedger};
use gent_types::{
    AgentChatConversationId, ContextPolicy, ConversationContentEntry, ConversationContentPage,
    NormalizedTranscriptAppend, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    NormalizedTranscriptPage,
};

use crate::{ConversationContextArtifactService, ConversationContextRequest};

struct Reader;

impl ConversationContentReader for Reader {
    fn read_conversation_content(
        &self,
        _: &str,
        _: Option<u64>,
        _: u16,
    ) -> Result<ConversationContentPage, LedgerError> {
        Ok(ConversationContentPage {
            conversation_id: "conversation".into(),
            entries: vec![ConversationContentEntry {
                message_id: "message".into(),
                turn_id: "turn".into(),
                run_id: "run".into(),
                ordinal: 1,
                text: "one".into(),
                text_digest_sha256: "a".repeat(64),
            }],
            next_before: None,
        })
    }
}

impl TranscriptLedger for Reader {
    fn append_normalized_transcript(
        &self,
        _: &AgentChatConversationId,
        _: &NormalizedTranscriptAppend,
    ) -> Result<NormalizedTranscriptEvent, LedgerError> {
        unreachable!("read-only test")
    }

    fn normalized_transcript_page(
        &self,
        _: &AgentChatConversationId,
        after: u64,
        _: u16,
    ) -> Result<NormalizedTranscriptPage, LedgerError> {
        let event = |cursor: u64, turn_id: &str, kind: NormalizedTranscriptKind| {
            NormalizedTranscriptEvent {
                cursor,
                event_id: format!("event-{cursor}"),
                turn_id: turn_id.into(),
                run_id: "run".into(),
                kind,
                text: "text".into(),
                is_partial: false,
            }
        };
        Ok(match after {
            0 => NormalizedTranscriptPage {
                conversation_id: "conversation".into(),
                events: vec![event(1, "turn", NormalizedTranscriptKind::AssistantMessage)],
                next_after_cursor: Some(1),
            },
            1 => NormalizedTranscriptPage {
                conversation_id: "conversation".into(),
                events: vec![event(2, "other", NormalizedTranscriptKind::Notice)],
                next_after_cursor: None,
            },
            _ => return Err(LedgerError::Invariant("unexpected cursor".into())),
        })
    }
}

#[test]
fn preserve_resumes_strictly_after_the_last_transcript_cursor() {
    let artifact = ConversationContextArtifactService::new(Reader)
        .project(&ConversationContextRequest {
            conversation_id: AgentChatConversationId("conversation".into()),
            context_policy: ContextPolicy::Preserve,
            context_through_ordinal: 1,
        })
        .unwrap();
    assert_eq!(artifact.transcript_events.len(), 1);
    assert_eq!(artifact.transcript_events[0].cursor, 1);
}
