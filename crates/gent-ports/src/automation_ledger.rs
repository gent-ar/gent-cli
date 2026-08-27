use gent_types::{AutomationDefinition, AutomationId, AutomationRun, AutomationRunId};

use crate::LedgerError;

pub trait AutomationLedger: Send + Sync {
    fn create_automation(&self, definition: &AutomationDefinition) -> Result<(), LedgerError>;
    fn find_automation(
        &self,
        automation_id: &AutomationId,
    ) -> Result<Option<AutomationDefinition>, LedgerError>;
    fn list_automations(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<AutomationDefinition>, LedgerError>;
    fn record_automation_run(&self, run: &AutomationRun) -> Result<(), LedgerError>;
    fn find_automation_run(
        &self,
        run_id: &AutomationRunId,
    ) -> Result<Option<AutomationRun>, LedgerError>;
    fn list_automation_runs(
        &self,
        automation_id: &AutomationId,
        limit: u16,
    ) -> Result<Vec<AutomationRun>, LedgerError>;
}
