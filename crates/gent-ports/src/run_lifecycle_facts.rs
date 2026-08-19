//! Immutable, replayable lifecycle facts owned by the daemon ledger.

use gent_types::{RunLifecycleFact, RunLifecycleFactPage};

use crate::LedgerError;

/// Appends and pages normalized run lifecycle facts; it never stores reduced state.
pub trait RunLifecycleFactLedger: Send + Sync {
    /// Appends an exact lifecycle fact or accepts the identical durable retry.
    ///
    /// # Errors
    /// Returns an error when the source cursor, run identity, or retry conflicts.
    fn append_run_lifecycle_fact(&self, fact: &RunLifecycleFact) -> Result<(), LedgerError>;

    /// Reads a bounded, cursor-ordered page for one run.
    ///
    /// # Errors
    /// Returns an error when the page cannot be read.
    fn read_run_lifecycle_fact_page(
        &self,
        run_id: &str,
        after_cursor: u64,
        limit: usize,
    ) -> Result<RunLifecycleFactPage, LedgerError>;
}
