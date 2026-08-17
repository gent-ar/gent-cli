//! Read-only durable boundary for normalized agent-chat views.

use gent_types::{
    AgentChatConversationDetail, AgentChatConversationSummary, NormalizedTranscriptPage,
};

use crate::LedgerError;

/// Reads provider-neutral conversation metadata and normalized transcript pages.
///
/// Implementations must never return provider-native session identifiers, raw provider payloads,
/// credentials, or unbounded transcript data.
pub trait AgentChatReadLedger: Send + Sync {
    /// Reads the public summary for one durable conversation.
    ///
    /// # Errors
    /// Returns an error when durable read storage is unavailable or the conversation is unknown.
    fn read_agent_chat_summary(
        &self,
        conversation_id: &str,
    ) -> Result<AgentChatConversationSummary, LedgerError>;

    /// Reads the public run hierarchy for one durable conversation.
    ///
    /// # Errors
    /// Returns an error when durable read storage is unavailable or the conversation is unknown.
    fn read_agent_chat_detail(
        &self,
        conversation_id: &str,
    ) -> Result<AgentChatConversationDetail, LedgerError>;

    /// Reads an ascending, cursor-paginated normalized transcript page.
    ///
    /// # Errors
    /// Returns an error when durable read storage is unavailable or the cursor is invalid.
    fn read_agent_chat_transcript(
        &self,
        conversation_id: &str,
        after_cursor: Option<u64>,
        limit: u16,
    ) -> Result<NormalizedTranscriptPage, LedgerError>;
}
