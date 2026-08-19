//! Replay-only run status derivation over immutable normalized lifecycle facts.

use gent_core::{
    LifecycleProjection, project_lifecycle_signal, project_normalized_event, projected_live_status,
};
use gent_ports::{Ledger, RunLifecycleFactLedger};
use gent_types::{NormalizedSessionLifecycle, RunLifecycleFact, RunLiveStatus};

use crate::{Coordinator, RuntimeError};

const FACT_PAGE_SIZE: usize = 128;

/// Derives status from durable facts without retaining a mutable projection.
#[derive(Debug)]
pub struct RunLifecycleStatusService<L> {
    coordinator: Coordinator<L>,
}

impl<L> RunLifecycleStatusService<L>
where
    L: Ledger + RunLifecycleFactLedger,
{
    /// Creates a replay-only status reader.
    #[must_use]
    pub fn new(coordinator: Coordinator<L>) -> Self {
        Self { coordinator }
    }

    /// Replays all bounded pages for one run to derive its current lifecycle state.
    ///
    /// # Errors
    /// Returns an error when a fact page is malformed or cannot be read.
    pub fn live_status(&self, run_id: &str) -> Result<Option<RunLiveStatus>, RuntimeError> {
        let mut cursor = 0;
        let mut state = LifecycleProjection::default();
        let mut host_epoch = None;
        loop {
            let page = self.coordinator.ledger.read_run_lifecycle_fact_page(
                run_id,
                cursor,
                FACT_PAGE_SIZE,
            )?;
            for fact in &page.facts {
                apply(&mut state, fact);
                host_epoch = Some(fact.host_epoch);
            }
            let Some(next) = page.next_after_cursor else {
                break;
            };
            if next <= cursor {
                return Err(RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
                    "lifecycle fact page cursor did not advance".into(),
                )));
            }
            cursor = next;
        }
        Ok(host_epoch.map(|host_epoch| RunLiveStatus {
            run_id: run_id.into(),
            host_epoch,
            status: projected_live_status(&state),
        }))
    }
}

fn apply(state: &mut LifecycleProjection, fact: &RunLifecycleFact) {
    *state = match &fact.lifecycle {
        NormalizedSessionLifecycle::Event { event } => {
            project_normalized_event(state.clone(), fact.cursor, event).state
        }
        NormalizedSessionLifecycle::Signal { signal } => {
            project_lifecycle_signal(state.clone(), fact.cursor, signal).state
        }
    };
}
