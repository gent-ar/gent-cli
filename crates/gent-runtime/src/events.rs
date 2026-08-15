use gent_ports::Ledger;
use gent_types::{EventResume, EventSnapshot};

use crate::{Coordinator, RuntimeError};

impl<L: Ledger> Coordinator<L> {
    /// Safely resumes events, replacing a stale client projection when compaction requires it.
    ///
    /// # Errors
    /// Returns an error when durable event state cannot be read.
    pub fn resume_events(&self, cursor: u64) -> Result<EventResume, RuntimeError> {
        Ok(self.ledger.resume_events(cursor)?)
    }

    /// Commits a projection-owned snapshot and retires its covered durable event prefix.
    ///
    /// # Errors
    /// Returns an error when the snapshot is not a monotonic prefix of the event log.
    pub fn compact_events(&self, snapshot: &EventSnapshot) -> Result<(), RuntimeError> {
        Ok(self.ledger.compact_events(snapshot)?)
    }
}
