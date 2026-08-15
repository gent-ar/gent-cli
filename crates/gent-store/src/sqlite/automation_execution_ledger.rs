//! Adapter joining automation execution records to the public persistence port.

use gent_ports::{AutomationExecutionLedger, AutomationExecutionUpdate, LedgerError};
use gent_types::{AutomationExecutionPhase, AutomationExecutionRecord};

use super::{SqliteLedger, automation_executions};

impl AutomationExecutionLedger for SqliteLedger {
    fn create_automation_execution(
        &self,
        execution: &AutomationExecutionRecord,
    ) -> Result<(), LedgerError> {
        automation_executions::create(self, execution)
    }

    fn find_automation_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<AutomationExecutionRecord>, LedgerError> {
        automation_executions::find(self, execution_id)
    }

    fn list_automation_executions(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<AutomationExecutionRecord>, LedgerError> {
        automation_executions::list(self, workspace_id)
    }

    fn replace_automation_execution_phase(
        &self,
        execution_id: &str,
        expected: AutomationExecutionPhase,
        next: AutomationExecutionPhase,
    ) -> Result<AutomationExecutionUpdate, LedgerError> {
        automation_executions::replace_phase(self, execution_id, expected, next)
    }
}
