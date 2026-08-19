//! Durable boundary for immutable, conversation-scoped activity facts.

use gent_types::{ConversationActivityFact, ConversationActivityPage};

use crate::LedgerError;

/// Maximum facts returned by one activity-history page.
pub const MAX_CONVERSATION_ACTIVITY_PAGE_FACTS: usize = 64;

/// Persistence for canonical conversation activity facts.
pub trait ConversationActivityLedger: Send + Sync {
    /// Appends one assigned, immutable activity fact, preserving an identical retry.
    ///
    /// # Errors
    /// Returns an error when lineage or ordering invariants fail, or storage fails.
    fn append_conversation_activity(
        &self,
        fact: &ConversationActivityFact,
    ) -> Result<(), LedgerError>;

    /// Reads one bounded, cursor-ordered page strictly after `after_cursor`.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn read_conversation_activity_page(
        &self,
        conversation_id: &str,
        run_id: &str,
        after_cursor: u64,
        limit: usize,
    ) -> Result<ConversationActivityPage, LedgerError>;
}
