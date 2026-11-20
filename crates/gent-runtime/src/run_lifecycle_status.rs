use gent_ports::{Ledger, RunLifecycleFactLedger};
use gent_types::RunLiveStatus;

use crate::{Coordinator, RuntimeError};

#[derive(Debug)]
pub struct RunLifecycleStatusService<L> {
    coordinator: Coordinator<L>,
}

impl<L> RunLifecycleStatusService<L>
where
    L: Ledger + RunLifecycleFactLedger,
{
    #[must_use]
    pub fn new(coordinator: Coordinator<L>) -> Self {
        Self { coordinator }
    }

    pub fn live_status(&self, run_id: &str) -> Result<Option<RunLiveStatus>, RuntimeError> {
        Ok(self.coordinator.ledger.run_live_status(run_id)?)
    }
}
