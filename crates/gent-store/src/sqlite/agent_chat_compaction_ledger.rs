//! Indexed canonical-event read for one run's normalized compaction facts.

use gent_ports::{AgentChatCompactionLedger, LedgerError};
use gent_types::EventPage;

use super::{SqliteLedger, event_pages};

impl AgentChatCompactionLedger for SqliteLedger {
    fn read_agent_chat_compaction_page(
        &self,
        run_id: &str,
        after_cursor: u64,
        limit: usize,
    ) -> Result<EventPage, LedgerError> {
        event_pages::read_compaction(&*self.lock()?, run_id, after_cursor, limit)
    }
}
