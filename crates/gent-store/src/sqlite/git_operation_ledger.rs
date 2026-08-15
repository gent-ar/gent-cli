//! Adapter joining Git operation records to the public persistence port.

use gent_ports::{GitOperationLedger, GitOperationUpdate, LedgerError};
use gent_types::{GitOperationPhase, GitOperationRecord};

use super::{SqliteLedger, git_operations};

impl GitOperationLedger for SqliteLedger {
    fn create_git_operation(&self, operation: &GitOperationRecord) -> Result<(), LedgerError> {
        git_operations::create(self, operation)
    }

    fn find_git_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<GitOperationRecord>, LedgerError> {
        git_operations::find(self, operation_id)
    }

    fn replace_git_operation_phase(
        &self,
        operation_id: &str,
        expected: GitOperationPhase,
        next: GitOperationPhase,
    ) -> Result<GitOperationUpdate, LedgerError> {
        git_operations::replace_phase(self, operation_id, expected, next)
    }
}
