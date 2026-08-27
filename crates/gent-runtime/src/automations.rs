use gent_ports::AutomationLedger;
use gent_types::{AutomationDefinition, AutomationId, AutomationRun, AutomationRunId};

use crate::RuntimeError;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AutomationAuthority {
    #[default]
    Observer,
    Approved,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutomationResult {
    DeniedObserver,
    Missing,
    Definition(AutomationDefinition),
    Definitions(Vec<AutomationDefinition>),
    Run(AutomationRun),
    Runs(Vec<AutomationRun>),
    Recorded,
}

#[derive(Clone, Debug)]
pub struct AutomationService<L> {
    ledger: L,
    authority: AutomationAuthority,
}

impl<L> AutomationService<L> {
    #[must_use]
    pub fn new(ledger: L, authority: AutomationAuthority) -> Self {
        Self { ledger, authority }
    }
}

impl<L: AutomationLedger> AutomationService<L> {
    pub fn create(
        &self,
        definition: AutomationDefinition,
    ) -> Result<AutomationResult, RuntimeError> {
        if self.authority != AutomationAuthority::Approved {
            return Ok(AutomationResult::DeniedObserver);
        }
        self.ledger.create_automation(&definition)?;
        Ok(AutomationResult::Definition(definition))
    }

    pub fn list(&self, workspace_id: &str) -> Result<AutomationResult, RuntimeError> {
        if self.authority != AutomationAuthority::Approved {
            return Ok(AutomationResult::DeniedObserver);
        }
        Ok(AutomationResult::Definitions(
            self.ledger.list_automations(workspace_id)?,
        ))
    }

    pub fn get(&self, automation_id: &AutomationId) -> Result<AutomationResult, RuntimeError> {
        if self.authority != AutomationAuthority::Approved {
            return Ok(AutomationResult::DeniedObserver);
        }
        Ok(self
            .ledger
            .find_automation(automation_id)?
            .map_or(AutomationResult::Missing, AutomationResult::Definition))
    }

    pub fn record_run(&self, run: AutomationRun) -> Result<AutomationResult, RuntimeError> {
        if self.authority != AutomationAuthority::Approved {
            return Ok(AutomationResult::DeniedObserver);
        }
        self.ledger.record_automation_run(&run)?;
        Ok(AutomationResult::Recorded)
    }

    pub fn runs(
        &self,
        automation_id: &AutomationId,
        limit: u16,
    ) -> Result<AutomationResult, RuntimeError> {
        if self.authority != AutomationAuthority::Approved {
            return Ok(AutomationResult::DeniedObserver);
        }
        Ok(AutomationResult::Runs(
            self.ledger.list_automation_runs(automation_id, limit)?,
        ))
    }

    pub fn run(&self, run_id: &AutomationRunId) -> Result<AutomationResult, RuntimeError> {
        if self.authority != AutomationAuthority::Approved {
            return Ok(AutomationResult::DeniedObserver);
        }
        Ok(self
            .ledger
            .find_automation_run(run_id)?
            .map_or(AutomationResult::Missing, AutomationResult::Run))
    }
}
