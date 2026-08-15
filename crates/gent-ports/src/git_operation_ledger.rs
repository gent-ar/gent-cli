//! Durable boundary for worktree-scoped Git operation lifecycle records.

use gent_types::{GitOperationPhase, GitOperationRecord};

use crate::LedgerError;

/// Result of an optimistic Git operation phase update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitOperationUpdate {
    Applied(GitOperationRecord),
    Current(GitOperationRecord),
}

/// Persistence boundary for operation records; Git execution does not belong here.
pub trait GitOperationLedger: Send + Sync {
    /// Creates a worktree and run-bound operation in its requested phase.
    ///
    /// # Errors
    /// Returns an error when hierarchy invariants fail or persistence fails.
    fn create_git_operation(&self, operation: &GitOperationRecord) -> Result<(), LedgerError>;

    /// Reads an operation record by immutable identity.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_git_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<GitOperationRecord>, LedgerError>;

    /// Updates phase only if it still equals the expected phase.
    ///
    /// # Errors
    /// Returns an error when the operation is missing or durable persistence fails.
    fn replace_git_operation_phase(
        &self,
        operation_id: &str,
        expected: GitOperationPhase,
        next: GitOperationPhase,
    ) -> Result<GitOperationUpdate, LedgerError>;
}
