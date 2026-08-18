use std::sync::{Arc, Mutex};

use gent_ports::{ConversationContentReader, LedgerError, TranscriptLedger};
use gent_types::{
    ContextPolicy, ConversationContentCursor, ConversationContentEntry, ConversationContentPage,
    NormalizedTranscriptAppend, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    NormalizedTranscriptPage,
};

use crate::{ConversationContextArtifactService, ConversationContextRequest};

type ContentPages = Arc<Mutex<Vec<(Option<u64>, ConversationContentPage)>>>;
type TranscriptPages = Arc<Mutex<Vec<(u64, NormalizedTranscriptPage)>>>;

#[derive(Clone, Default)]
struct Reader {
    content: ContentPages,
    transcript: TranscriptPages,
    reads: Arc<Mutex<u8>>,
}

impl ConversationContentReader for Reader {
    fn read_conversation_content(
        &self,
        _: &str,
        before: Option<u64>,
        _: u16,
    ) -> Result<ConversationContentPage, LedgerError> {
        *self.reads.lock().unwrap() += 1;
        find_content(&self.content, before)
    }
}

impl TranscriptLedger for Reader {
    fn append_normalized_transcript(
        &self,
        _: &gent_types::AgentChatConversationId,
        _: &NormalizedTranscriptAppend,
    ) -> Result<NormalizedTranscriptEvent, LedgerError> {
        unreachable!("context projection never writes transcripts")
    }

    fn normalized_transcript_page(
        &self,
        _: &gent_types::AgentChatConversationId,
        after: u64,
        _: u16,
    ) -> Result<NormalizedTranscriptPage, LedgerError> {
        *self.reads.lock().unwrap() += 1;
        find_transcript(&self.transcript, after)
    }
}

#[test]
fn clear_is_empty_and_never_reads_history() {
    let reader = Reader::default();
    let artifact = ConversationContextArtifactService::new(reader.clone())
        .project(&request(ContextPolicy::Clear, 0))
        .unwrap();
    assert!(artifact.entries.is_empty());
    assert!(artifact.transcript_events.is_empty());
    assert_eq!(artifact.context_through_ordinal, 0);
    assert_eq!(*reader.reads.lock().unwrap(), 0);
    assert!(
        ConversationContextArtifactService::new(reader)
            .project(&request(ContextPolicy::Clear, 2))
            .is_err()
    );
}

#[test]
fn preserve_collects_frozen_prompt_and_final_provider_neutral_history() {
    let reader = Reader {
        content: Arc::new(Mutex::new(vec![
            (
                Some(4),
                content_page(vec![entry(3, "three"), entry(2, "two")], Some(2)),
            ),
            (Some(2), content_page(vec![entry(1, "one")], None)),
        ])),
        transcript: Arc::new(Mutex::new(vec![(
            0,
            transcript_page(vec![
                transcript(
                    1,
                    "turn-1",
                    NormalizedTranscriptKind::AssistantMessage,
                    false,
                ),
                transcript(
                    2,
                    "turn-2",
                    NormalizedTranscriptKind::AssistantMessage,
                    true,
                ),
                transcript(3, "turn-3", NormalizedTranscriptKind::ToolActivity, false),
                transcript(4, "other", NormalizedTranscriptKind::Notice, false),
                transcript(5, "turn-3", NormalizedTranscriptKind::UserMessage, false),
            ]),
        )])),
        reads: Arc::new(Mutex::new(0)),
    };
    let artifact = ConversationContextArtifactService::new(reader)
        .project(&request(ContextPolicy::Preserve, 3))
        .unwrap();
    assert_eq!(
        artifact
            .entries
            .iter()
            .map(|entry| entry.ordinal)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        artifact
            .transcript_events
            .iter()
            .map(|event| event.cursor)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(artifact.content_digest_sha256.len(), 64);
}

#[test]
fn preserve_rejects_a_page_beyond_its_frozen_boundary() {
    let reader = Reader {
        content: Arc::new(Mutex::new(vec![(
            Some(3),
            content_page(vec![entry(3, "future")], None),
        )])),
        ..Reader::default()
    };
    assert!(
        ConversationContextArtifactService::new(reader)
            .project(&request(ContextPolicy::Preserve, 2))
            .is_err()
    );
}

#[test]
fn preserve_zero_is_empty_but_malformed_transcript_pages_are_rejected() {
    let reader = Reader::default();
    let empty = ConversationContextArtifactService::new(reader.clone())
        .project(&request(ContextPolicy::Preserve, 0))
        .unwrap();
    assert!(empty.entries.is_empty());
    assert_eq!(*reader.reads.lock().unwrap(), 0);

    let malformed = Reader {
        content: Arc::new(Mutex::new(vec![(
            Some(2),
            content_page(vec![entry(1, "one")], None),
        )])),
        transcript: Arc::new(Mutex::new(vec![(
            0,
            NormalizedTranscriptPage {
                conversation_id: "another-conversation".into(),
                events: Vec::new(),
                next_after_cursor: None,
            },
        )])),
        reads: Arc::new(Mutex::new(0)),
    };
    assert!(
        ConversationContextArtifactService::new(malformed)
            .project(&request(ContextPolicy::Preserve, 1))
            .is_err()
    );
}

#[test]
fn preserve_rejects_non_advancing_transcript_events_and_continuations() {
    for page in [
        transcript_page(vec![transcript(
            0,
            "turn-1",
            NormalizedTranscriptKind::AssistantMessage,
            false,
        )]),
        NormalizedTranscriptPage {
            conversation_id: "conversation".into(),
            events: vec![transcript(
                1,
                "turn-1",
                NormalizedTranscriptKind::AssistantMessage,
                false,
            )],
            next_after_cursor: Some(1),
        },
    ] {
        let reader = Reader {
            content: Arc::new(Mutex::new(vec![(
                Some(2),
                content_page(vec![entry(1, "one")], None),
            )])),
            transcript: Arc::new(Mutex::new(vec![(0, page)])),
            reads: Arc::new(Mutex::new(0)),
        };
        assert!(
            ConversationContextArtifactService::new(reader)
                .project(&request(ContextPolicy::Preserve, 1))
                .is_err()
        );
    }
}

fn find_content(
    pages: &ContentPages,
    before: Option<u64>,
) -> Result<ConversationContentPage, LedgerError> {
    pages
        .lock()
        .unwrap()
        .iter()
        .find(|(expected, _)| *expected == before)
        .map(|(_, page)| page.clone())
        .ok_or_else(|| LedgerError::Invariant("unexpected context read".into()))
}

fn find_transcript(
    pages: &TranscriptPages,
    after: u64,
) -> Result<NormalizedTranscriptPage, LedgerError> {
    pages
        .lock()
        .unwrap()
        .iter()
        .find(|(expected, _)| *expected == after)
        .map(|(_, page)| page.clone())
        .ok_or_else(|| LedgerError::Invariant("unexpected transcript read".into()))
}

fn request(policy: ContextPolicy, ordinal: u64) -> ConversationContextRequest {
    ConversationContextRequest {
        conversation_id: gent_types::AgentChatConversationId("conversation".into()),
        context_policy: policy,
        context_through_ordinal: ordinal,
    }
}

fn content_page(
    entries: Vec<ConversationContentEntry>,
    next: Option<u64>,
) -> ConversationContentPage {
    ConversationContentPage {
        conversation_id: "conversation".into(),
        entries,
        next_before: next.map(|ordinal| ConversationContentCursor::new("conversation", ordinal)),
    }
}

fn transcript_page(events: Vec<NormalizedTranscriptEvent>) -> NormalizedTranscriptPage {
    NormalizedTranscriptPage {
        conversation_id: "conversation".into(),
        events,
        next_after_cursor: None,
    }
}

fn entry(ordinal: u64, text: &str) -> ConversationContentEntry {
    ConversationContentEntry {
        message_id: format!("message-{ordinal}"),
        turn_id: format!("turn-{ordinal}"),
        run_id: "run".into(),
        ordinal,
        text: text.into(),
        text_digest_sha256: "a".repeat(64),
    }
}

fn transcript(
    cursor: u64,
    turn_id: &str,
    kind: NormalizedTranscriptKind,
    is_partial: bool,
) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("event-{cursor}"),
        turn_id: turn_id.into(),
        run_id: "run".into(),
        kind,
        text: format!("event-{cursor}"),
        is_partial,
    }
}
