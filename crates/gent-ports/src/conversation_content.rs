//! Read-only local content boundary, separate from mutable prompt persistence.

use gent_types::ConversationContentPage;

use crate::LedgerError;

pub trait ConversationContentReader: Send + Sync {
    /// Reads a bounded newest-first page before an exclusive immutable ordinal.
    ///
    /// # Errors
    /// Returns an error when the page cannot be read from durable storage.
    fn read_conversation_content(
        &self,
        conversation_id: &str,
        before_ordinal: Option<u64>,
        limit: u16,
    ) -> Result<ConversationContentPage, LedgerError>;
}
