use gent_ports::Ledger;
use gent_types::EventPage;

use crate::{Coordinator, RuntimeError};

impl<L: Ledger> Coordinator<L> {
    /// Reads a bounded page from the immutable cursor-ordered event log.
    ///
    /// # Errors
    /// Returns an error when durable event state cannot be read.
    pub fn read_event_page(
        &self,
        after_cursor: u64,
        limit: usize,
    ) -> Result<EventPage, RuntimeError> {
        Ok(self.ledger.read_event_page(after_cursor, limit)?)
    }
}
