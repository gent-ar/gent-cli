//! Coordinator orchestration for durable Git operation lifecycles.

use gent_core::permits_git_operation_transition;
use gent_ports::{GitOperationLedger, GitOperationUpdate, Ledger, LedgerError};
use gent_types::{GitOperationPhase, GitOperationRecord};

use crate::{Coordinator, RuntimeError};

impl<L> Coordinator<L>
where
    L: Ledger + GitOperationLedger,
{
    /// Persists a requested worktree-scoped Git operation without executing it.
    ///
    /// # Errors
    /// Returns an error when durable hierarchy invariants or persistence fail.
    pub fn create_git_operation(&self, operation: &GitOperationRecord) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_git_operation(operation)?)
    }

    /// Advances a Git operation through its pure monotonic lifecycle policy.
    ///
    /// # Errors
    /// Returns an error when the operation is missing, transition is invalid, or persistence fails.
    pub fn transition_git_operation(
        &self,
        operation_id: &str,
        expected: GitOperationPhase,
        next: GitOperationPhase,
    ) -> Result<GitOperationUpdate, RuntimeError> {
        if !permits_git_operation_transition(expected, next) {
            return Err(RuntimeError::Ledger(LedgerError::Invariant(
                "git operation transition is not permitted".into(),
            )));
        }
        Ok(self
            .ledger
            .replace_git_operation_phase(operation_id, expected, next)?)
    }
}
