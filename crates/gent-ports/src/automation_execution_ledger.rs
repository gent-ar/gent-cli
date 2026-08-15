//! Durable boundary for workspace-scoped automation execution records.

use gent_types::{AutomationExecutionPhase, AutomationExecutionRecord};

use crate::LedgerError;

/// Result of an optimistic automation execution phase update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutomationExecutionUpdate {
    Applied(AutomationExecutionRecord),
    Current(AutomationExecutionRecord),
}

/// Persistence boundary for automation executions; triggering work does not belong here.
pub trait AutomationExecutionLedger: Send + Sync {
    /// Creates a queued execution under an existing workspace.
    ///
    /// # Errors
    /// Returns an error when identity or deduplication invariants fail.
    fn create_automation_execution(
        &self,
        execution: &AutomationExecutionRecord,
    ) -> Result<(), LedgerError>;

    /// Reads one execution by immutable identity.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_automation_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<AutomationExecutionRecord>, LedgerError>;

    /// Lists a workspace's executions in durable creation order.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn list_automation_executions(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<AutomationExecutionRecord>, LedgerError>;

    /// Updates phase only if it still equals the expected phase.
    ///
    /// # Errors
    /// Returns an error when the execution is missing or persistence fails.
    fn replace_automation_execution_phase(
        &self,
        execution_id: &str,
        expected: AutomationExecutionPhase,
        next: AutomationExecutionPhase,
    ) -> Result<AutomationExecutionUpdate, LedgerError>;
}
