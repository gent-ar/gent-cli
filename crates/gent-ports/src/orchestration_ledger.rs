//! Durable boundary for future Gent-owned fanout and cross-review graphs.

use gent_types::{CrossReviewRequest, FanoutRequest, TaskGraph};

use crate::LedgerError;

/// Result of one atomic graph command after durable fence rechecks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrchestrationWrite {
    Created(TaskGraph),
    Updated(TaskGraph),
    Current(TaskGraph),
}

/// Persistence boundary for typed orchestration state; it never launches a provider or worktree.
pub trait OrchestrationLedger: Send + Sync {
    /// Reads one graph only by its immutable Gent-owned identity.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn task_graph(&self, graph_id: &str) -> Result<Option<TaskGraph>, LedgerError>;
    /// Atomically creates the initial typed fanout graph or returns its identical durable retry.
    ///
    /// # Errors
    /// Returns an error when an exact durable fence or idempotency check fails.
    fn apply_fanout(&self, request: &FanoutRequest) -> Result<OrchestrationWrite, LedgerError>;
    /// Atomically rechecks and appends one exact cross-provider review node.
    ///
    /// # Errors
    /// Returns an error when an exact durable fence or idempotency check fails.
    fn apply_cross_review(
        &self,
        request: &CrossReviewRequest,
    ) -> Result<OrchestrationWrite, LedgerError>;
}
