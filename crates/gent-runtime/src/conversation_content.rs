//! Read-only conversation content coordinator boundary.

use gent_ports::ConversationContentReader;
use gent_types::{ConversationContentCursor, ConversationContentPage};

use crate::{Coordinator, RuntimeError};

impl<L> Coordinator<L>
where
    L: gent_ports::Ledger + ConversationContentReader,
{
    /// Reads a bounded newest-first page of user-authored content without a receipt or side effect.
    ///
    /// # Errors
    /// Returns an error for malformed cursors or unavailable durable content storage.
    pub fn conversation_content(
        &self,
        conversation_id: &str,
        before: Option<&ConversationContentCursor>,
        limit: u16,
    ) -> Result<ConversationContentPage, RuntimeError> {
        let before = before
            .map(|cursor| cursor.ordinal_for(conversation_id))
            .transpose()
            .map_err(|error| {
                RuntimeError::Ledger(gent_ports::LedgerError::Invariant(error.to_string()))
            })?;
        Ok(self
            .ledger
            .read_conversation_content(conversation_id, before, limit.clamp(1, 100))?)
    }
}
