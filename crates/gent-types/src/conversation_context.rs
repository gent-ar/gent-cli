//! Provider-neutral, frozen local context prepared for a child conversation run.
//!
//! This private-local value deliberately contains no provider-native session, frame, credential,
//! endpoint, or hidden reasoning. Its ordinal is a durable boundary chosen when the child run is
//! created; a clear context is represented by ordinal zero and an empty entry list.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    AgentChatConversationId, ConversationContentEntry, NormalizedTranscriptEvent,
    NormalizedTranscriptKind,
};

/// Bounded durable history that a daemon-owned lifecycle may render for a fresh provider session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrozenConversationContext {
    pub conversation_id: AgentChatConversationId,
    pub context_through_ordinal: u64,
    pub entries: Vec<ConversationContentEntry>,
    pub transcript_events: Vec<NormalizedTranscriptEvent>,
    /// Digest over every transcript field rendered into provider input.
    pub transcript_digest_sha256: String,
    pub content_digest_sha256: String,
}

impl FrozenConversationContext {
    /// Returns the canonical empty artifact used for explicitly cleared context.
    #[must_use]
    pub fn cleared(conversation_id: AgentChatConversationId) -> Self {
        Self {
            conversation_id,
            context_through_ordinal: 0,
            entries: Vec::new(),
            transcript_events: Vec::new(),
            transcript_digest_sha256: "0".repeat(64),
            content_digest_sha256: "0".repeat(64),
        }
    }

    /// Calculates the canonical digest for rendered normalized transcript history.
    #[must_use]
    pub fn transcript_digest(events: &[NormalizedTranscriptEvent]) -> String {
        let mut hasher = Sha256::new();
        for event in events {
            hasher.update(event.cursor.to_be_bytes());
            update_string(&mut hasher, &event.event_id);
            update_string(&mut hasher, &event.turn_id);
            update_string(&mut hasher, &event.run_id);
            hasher.update([kind_tag(event.kind)]);
            hasher.update([u8::from(event.is_partial)]);
            update_string(&mut hasher, &event.text);
        }
        format!("{:x}", hasher.finalize())
    }
}

fn update_string(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

const fn kind_tag(kind: NormalizedTranscriptKind) -> u8 {
    match kind {
        NormalizedTranscriptKind::UserMessage => 1,
        NormalizedTranscriptKind::AssistantMessage => 2,
        NormalizedTranscriptKind::Thinking => 3,
        NormalizedTranscriptKind::ToolActivity => 4,
        NormalizedTranscriptKind::Notice => 5,
    }
}
