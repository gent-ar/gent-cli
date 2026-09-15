use gent_ports::{ConversationContentReader, LedgerError, TranscriptLedger};
use gent_types::{
    AgentChatConversationId, AgentChatRunContext, AgentChatRunContextOrigin, AgentChatRunId,
    ContextPolicy, ConversationContentCursor, ConversationContentEntry, ConversationContentPage,
    NormalizedTranscriptAppend, NormalizedTranscriptEvent, NormalizedTranscriptKind,
    NormalizedTranscriptPage,
};

use crate::{ConversationContextArtifactService, ConversationContextRequest};

struct History {
    entries: Vec<ConversationContentEntry>,
    events: Vec<NormalizedTranscriptEvent>,
}

impl History {
    fn new(turns: u64, text_bytes: usize) -> Self {
        let text = "x".repeat(text_bytes);
        Self {
            entries: (1..=turns)
                .map(|ordinal| ConversationContentEntry {
                    message_id: format!("message-{ordinal}"),
                    turn_id: format!("turn-{ordinal}"),
                    run_id: "run".into(),
                    ordinal,
                    text: format!("prompt-{ordinal}"),
                    text_digest_sha256: "a".repeat(64),
                })
                .collect(),
            events: (1..=turns)
                .map(|ordinal| NormalizedTranscriptEvent {
                    cursor: ordinal,
                    event_id: format!("event-{ordinal}"),
                    turn_id: format!("turn-{ordinal}"),
                    run_id: "run".into(),
                    kind: NormalizedTranscriptKind::ToolActivity,
                    text: format!("{ordinal}:{text}"),
                    is_partial: false,
                    origin: None,
                    attachments: Vec::new(),
                })
                .collect(),
        }
    }
}

impl History {
    fn switched_after(mut self, parent_turns: u64) -> Self {
        for entry in &mut self.entries {
            if entry.ordinal > parent_turns {
                entry.run_id = "child".into();
            }
        }
        self
    }
}

impl ConversationContentReader for History {
    fn read_conversation_content(
        &self,
        conversation_id: &str,
        before: Option<u64>,
        limit: u16,
    ) -> Result<ConversationContentPage, LedgerError> {
        let before = before.unwrap_or(u64::MAX);
        let entries = self
            .entries
            .iter()
            .rev()
            .filter(|entry| entry.ordinal < before)
            .take(usize::from(limit))
            .cloned()
            .collect::<Vec<_>>();
        let next_before = entries
            .last()
            .filter(|last| last.ordinal > 1)
            .map(|last| ConversationContentCursor::new(conversation_id, last.ordinal));
        Ok(ConversationContentPage {
            conversation_id: conversation_id.into(),
            entries,
            next_before,
        })
    }
}

impl TranscriptLedger for History {
    fn append_normalized_transcript(
        &self,
        _: &AgentChatConversationId,
        _: &NormalizedTranscriptAppend,
    ) -> Result<NormalizedTranscriptEvent, LedgerError> {
        unreachable!("context projection never writes transcripts")
    }

    fn normalized_transcript_page(
        &self,
        conversation_id: &AgentChatConversationId,
        after: u64,
        limit: u16,
    ) -> Result<NormalizedTranscriptPage, LedgerError> {
        let events = self
            .events
            .iter()
            .filter(|event| event.cursor > after)
            .take(usize::from(limit))
            .cloned()
            .collect::<Vec<_>>();
        let next_after_cursor = events
            .last()
            .map(|last| last.cursor)
            .filter(|cursor| self.events.iter().any(|event| event.cursor > *cursor));
        Ok(NormalizedTranscriptPage {
            conversation_id: conversation_id.0.clone(),
            events,
            next_after_cursor,
        })
    }
}

fn project(history: History, through: u64) -> gent_types::FrozenConversationContext {
    ConversationContextArtifactService::new(history)
        .project(&ConversationContextRequest {
            conversation_id: AgentChatConversationId("conversation".into()),
            context_policy: ContextPolicy::Preserve,
            context_through_ordinal: through,
        })
        .unwrap()
}

#[test]
fn long_history_keeps_the_newest_bounded_turns_instead_of_failing() {
    let artifact = project(History::new(250, 8), 250);
    assert_eq!(artifact.entries.len(), 200);
    assert_eq!(artifact.entries.first().unwrap().ordinal, 51);
    assert_eq!(artifact.entries.last().unwrap().ordinal, 250);
    assert_eq!(artifact.transcript_events.len(), 200);
    assert_eq!(
        artifact.transcript_events.first().unwrap().turn_id,
        "turn-51"
    );
    assert_eq!(
        artifact.transcript_events.last().unwrap().turn_id,
        "turn-250"
    );
}

#[test]
fn oversized_history_drops_the_oldest_turns_within_the_byte_bound() {
    let artifact = project(History::new(12, 60 * 1024), 12);
    assert!(serde_json::to_vec(&artifact).unwrap().len() <= 512 * 1024);
    assert_eq!(artifact.entries.last().unwrap().ordinal, 12);
    assert!(artifact.entries.first().unwrap().ordinal > 1);
    assert!(artifact.transcript_events.iter().all(|event| {
        artifact
            .entries
            .iter()
            .any(|entry| entry.turn_id == event.turn_id)
    }));
}

fn child_run(policy: ContextPolicy, through: u64) -> AgentChatRunContext {
    AgentChatRunContext {
        conversation_id: AgentChatConversationId("conversation".into()),
        run_id: AgentChatRunId("child".into()),
        origin: AgentChatRunContextOrigin::SelectionSwitch,
        context_policy: policy,
        context_through_ordinal: through,
    }
}

#[test]
fn a_fresh_session_in_a_switched_run_keeps_the_turns_made_since_the_switch() {
    let artifact = ConversationContextArtifactService::new(History::new(5, 8).switched_after(3))
        .project_run_before_message(&child_run(ContextPolicy::Preserve, 3), "message-5")
        .unwrap();
    assert_eq!(
        artifact
            .entries
            .iter()
            .map(|entry| entry.ordinal)
            .collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(artifact.context_through_ordinal, 4);
    assert_eq!(artifact.transcript_events.last().unwrap().turn_id, "turn-4");
}

#[test]
fn a_cleared_switched_run_keeps_only_its_own_earlier_turns() {
    let artifact = ConversationContextArtifactService::new(History::new(5, 8).switched_after(3))
        .project_run_before_message(&child_run(ContextPolicy::Clear, 0), "message-5")
        .unwrap();
    assert_eq!(
        artifact
            .entries
            .iter()
            .map(|entry| entry.ordinal)
            .collect::<Vec<_>>(),
        [4]
    );
    assert!(
        artifact
            .transcript_events
            .iter()
            .all(|event| event.turn_id == "turn-4")
    );
}

#[test]
fn an_interrupted_turn_keeps_its_partial_reply_marked_interrupted() {
    let mut history = History::new(2, 8);
    history.events = vec![
        NormalizedTranscriptEvent {
            cursor: 1,
            event_id: "delta-1".into(),
            turn_id: "turn-1".into(),
            run_id: "run".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "Half of ".into(),
            is_partial: true,
            origin: None,
            attachments: Vec::new(),
        },
        NormalizedTranscriptEvent {
            cursor: 2,
            event_id: "delta-2".into(),
            turn_id: "turn-1".into(),
            run_id: "run".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "the answer".into(),
            is_partial: true,
            origin: None,
            attachments: Vec::new(),
        },
        NormalizedTranscriptEvent {
            cursor: 3,
            event_id: "delta-3".into(),
            turn_id: "turn-2".into(),
            run_id: "run".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "Fin".into(),
            is_partial: true,
            origin: None,
            attachments: Vec::new(),
        },
        NormalizedTranscriptEvent {
            cursor: 4,
            event_id: "final-2".into(),
            turn_id: "turn-2".into(),
            run_id: "run".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "Finished".into(),
            is_partial: false,
            origin: None,
            attachments: Vec::new(),
        },
    ];
    let artifact = project(history, 2);
    let replies = artifact
        .transcript_events
        .iter()
        .map(|event| {
            (
                event.event_id.as_str(),
                event.text.as_str(),
                event.is_partial,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        replies,
        [
            ("interrupted:delta-2", "Half of the answer", false),
            ("final-2", "Finished", false)
        ]
    );
}
