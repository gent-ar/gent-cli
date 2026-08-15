//! Coordinator calls for durable automation execution lifecycles.

use gent_core::permits_automation_execution_transition;
use gent_ports::{AutomationExecutionLedger, AutomationExecutionUpdate, Ledger, LedgerError};
use gent_types::{AutomationExecutionPhase, AutomationExecutionRecord};

use crate::{Coordinator, RuntimeError};

impl<L> Coordinator<L>
where
    L: Ledger + AutomationExecutionLedger,
{
    /// Records one accepted trigger without starting a scheduler, webhook, or process.
    ///
    /// # Errors
    /// Returns an error when durable workspace or trigger invariants fail.
    pub fn create_automation_execution(
        &self,
        execution: &AutomationExecutionRecord,
    ) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_automation_execution(execution)?)
    }

    /// Advances an execution through its pure monotonic lifecycle policy.
    ///
    /// # Errors
    /// Returns an error when the transition is invalid or persistence fails.
    pub fn transition_automation_execution(
        &self,
        execution_id: &str,
        expected: AutomationExecutionPhase,
        next: AutomationExecutionPhase,
    ) -> Result<AutomationExecutionUpdate, RuntimeError> {
        if !permits_automation_execution_transition(expected, next) {
            return Err(RuntimeError::Ledger(LedgerError::Invariant(
                "automation execution transition is not permitted".into(),
            )));
        }
        Ok(self
            .ledger
            .replace_automation_execution_phase(execution_id, expected, next)?)
    }

    /// Lists durable execution state without evaluating automation triggers.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn automation_executions(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<AutomationExecutionRecord>, RuntimeError> {
        Ok(self.ledger.list_automation_executions(workspace_id)?)
    }
}
