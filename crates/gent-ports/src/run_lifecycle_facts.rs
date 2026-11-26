use gent_types::{RunLifecycleFact, RunLifecycleFactPage, RunLiveStatus};

use crate::LedgerError;

pub trait RunLifecycleFactLedger: Send + Sync {
    fn append_run_lifecycle_fact(&self, fact: &RunLifecycleFact) -> Result<(), LedgerError>;

    fn read_run_lifecycle_fact_page(
        &self,
        run_id: &str,
        after_cursor: u64,
        limit: usize,
    ) -> Result<RunLifecycleFactPage, LedgerError>;

    fn settle_run_permissions(&self, run_id: &str) -> Result<(), LedgerError>;

    fn run_live_status(&self, run_id: &str) -> Result<Option<RunLiveStatus>, LedgerError>;

    fn end_background_tools(
        &self,
        conversation_id: &str,
        phase: gent_types::ToolPhase,
    ) -> Result<(), LedgerError>;

    fn end_run_background_tools(
        &self,
        run_id: &str,
        phase: gent_types::ToolPhase,
    ) -> Result<(), LedgerError>;

    fn run_active_turn(&self, run_id: &str) -> Result<Option<String>, LedgerError>;
}
