//! Narrow read port over canonical compaction source events.

use gent_types::EventPage;

use crate::LedgerError;

/// Pages immutable normalized compaction events for one Gent run.
///
/// Implementations read the canonical event ledger. This is an indexed read shape, not a second
/// persistence stream, replay cache, or reduced-state store.
pub trait AgentChatCompactionLedger: Send + Sync {
    /// Reads one bounded, cursor-ordered page of canonical compaction facts for `run_id`.
    ///
    /// # Errors
    /// Returns an error when the durable source events cannot be read.
    fn read_agent_chat_compaction_page(
        &self,
        run_id: &str,
        after_cursor: u64,
        limit: usize,
    ) -> Result<EventPage, LedgerError>;
}
