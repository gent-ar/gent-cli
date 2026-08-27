//! Durable boundary for immutable per-conversation advanced-launch-configuration revisions.

use gent_types::AgentChatConversationConfigRecord;

use crate::LedgerError;

/// Persistence boundary for versioned per-conversation CLI-launch configuration.
pub trait AgentChatConversationConfigLedger: Send + Sync {
    /// Saves an immutable configuration revision under an existing conversation.
    ///
    /// # Errors
    /// Returns an error when the record is invalid, not next in sequence, or cannot persist.
    fn save_conversation_config(
        &self,
        config: &AgentChatConversationConfigRecord,
    ) -> Result<(), LedgerError>;

    /// Reads the latest configuration revision for one conversation.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn current_conversation_config(
        &self,
        conversation_id: &str,
    ) -> Result<Option<AgentChatConversationConfigRecord>, LedgerError>;
}
