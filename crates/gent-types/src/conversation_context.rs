//! Provider-neutral, frozen local context prepared for a child conversation run.
//!
//! This private-local value deliberately contains no provider-native session, frame, credential,
//! endpoint, or hidden reasoning. Its ordinal is a durable boundary chosen when the child run is
//! created; a clear context is represented by ordinal zero and an empty entry list.

use serde::{Deserialize, Serialize};

use crate::{AgentChatConversationId, ConversationContentEntry, NormalizedTranscriptEvent};

/// Bounded durable history that a daemon-owned lifecycle may render for a fresh provider session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrozenConversationContext {
    pub conversation_id: AgentChatConversationId,
    pub context_through_ordinal: u64,
    pub entries: Vec<ConversationContentEntry>,
    pub transcript_events: Vec<NormalizedTranscriptEvent>,
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
            content_digest_sha256: "0".repeat(64),
        }
    }
}
