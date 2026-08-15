//! Durable boundary for restart-safe, conversation-scoped activity projections.

use gent_types::ConversationActivityRecord;

use crate::LedgerError;

/// Persistence for complete conversation activity reducer state.
pub trait ConversationActivityLedger: Send + Sync {
    /// Saves one strictly newer complete state, preserving identical retries.
    ///
    /// # Errors
    /// Returns an error when lineage or ordering invariants fail, or storage fails.
    fn save_conversation_activity(
        &self,
        record: &ConversationActivityRecord,
    ) -> Result<(), LedgerError>;

    /// Reads the latest restart-safe activity state for one run in a conversation.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_conversation_activity(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Option<ConversationActivityRecord>, LedgerError>;
}
