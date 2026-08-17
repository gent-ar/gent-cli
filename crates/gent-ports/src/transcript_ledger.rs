//! Durable, cursor-ordered normalized transcript storage.

use gent_types::{
    AgentChatConversationId, NormalizedTranscriptAppend, NormalizedTranscriptEvent,
    NormalizedTranscriptPage,
};

use crate::LedgerError;

/// Persists provider-normalized transcript facts without granting provider authority.
///
/// The storage adapter assigns cursors and rejects changed retries for a producer-stable event
/// identity. Runtime composition remains responsible for deciding whether ingestion is allowed.
pub trait TranscriptLedger: Send + Sync {
    /// Appends one bounded normalized transcript fact or returns its exact prior durable result.
    ///
    /// # Errors
    /// Returns an error for an unknown hierarchy, changed idempotent retry, invalid content, or
    /// persistence failure.
    fn append_normalized_transcript(
        &self,
        conversation_id: &AgentChatConversationId,
        append: &NormalizedTranscriptAppend,
    ) -> Result<NormalizedTranscriptEvent, LedgerError>;

    /// Reads a bounded page of events strictly after a conversation-local durable cursor.
    ///
    /// # Errors
    /// Returns an error for invalid page bounds, an unknown conversation, or persistence failure.
    fn normalized_transcript_page(
        &self,
        conversation_id: &AgentChatConversationId,
        after_cursor: u64,
        limit: u16,
    ) -> Result<NormalizedTranscriptPage, LedgerError>;
}
