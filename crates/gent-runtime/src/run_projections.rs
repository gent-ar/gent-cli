//! Internal daemon orchestration for durable, lease-owned run lifecycle projections.

use gent_core::{
    project_lifecycle_signal, project_normalized_event, projected_live_status, restore_projection,
    snapshot_projection,
};
use gent_ports::{IngressMode, Ledger, RunProjectionLedger};
use gent_types::{
    HostEpoch, NormalizedLifecycleSignal, NormalizedProviderEvent, RunLiveStatus,
    RunProjectionRecord,
};

use crate::{Coordinator, RuntimeError};

/// Records and reads run-scoped lifecycle state without exposing provider-native session IDs.
#[derive(Debug)]
pub struct RunProjectionService<L> {
    coordinator: Coordinator<L>,
}

impl<L> RunProjectionService<L>
where
    L: Ledger + RunProjectionLedger,
{
    /// Creates the internal projection coordinator without starting any provider or transport.
    #[must_use]
    pub fn new(coordinator: Coordinator<L>) -> Self {
        Self { coordinator }
    }

    /// Applies one provider event at its durable cursor and saves the complete reduced projection.
    ///
    /// Callers must be the daemon's provider-event adapter. A matching run lease and
    /// server-owned provider session are required before any mutable projection is accepted. The
    /// caller is responsible for persisting the source event before invoking this reducer.
    ///
    /// # Errors
    /// Returns an error when the cursor is invalid, the host is fenced, or ownership is absent.
    pub fn record_normalized_event(
        &self,
        run_id: String,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        cursor: u64,
        event: &NormalizedProviderEvent,
    ) -> Result<RunLiveStatus, RuntimeError> {
        if cursor == 0 {
            return Err(invariant("provider events require a durable cursor"));
        }
        self.require_owner(&run_id, coordinator_id, host_epoch)?;
        let current = self
            .coordinator
            .ledger
            .find_run_projection(&run_id)?
            .map(|record| restore_projection(&record.projection))
            .unwrap_or_default();
        let update = project_normalized_event(current, cursor, event);
        let status = projected_live_status(&update.state);
        if update.applied {
            self.coordinator
                .ledger
                .save_run_projection(&RunProjectionRecord {
                    run_id: run_id.clone(),
                    host_epoch,
                    projection: snapshot_projection(&update.state),
                })?;
        }
        Ok(RunLiveStatus {
            run_id,
            host_epoch,
            status,
        })
    }

    /// Applies one adapter lifecycle signal at a durable cursor.
    ///
    /// It has the same ownership and session prerequisites as a content event.
    ///
    /// # Errors
    /// Returns an error when the cursor is invalid, the host is fenced, or ownership is absent.
    pub fn record_lifecycle_signal(
        &self,
        run_id: String,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        cursor: u64,
        signal: &NormalizedLifecycleSignal,
    ) -> Result<RunLiveStatus, RuntimeError> {
        if cursor == 0 {
            return Err(invariant("provider signals require a durable cursor"));
        }
        self.require_owner(&run_id, coordinator_id, host_epoch)?;
        let current = self
            .coordinator
            .ledger
            .find_run_projection(&run_id)?
            .map(|record| restore_projection(&record.projection))
            .unwrap_or_default();
        let update = project_lifecycle_signal(current, cursor, signal);
        let status = projected_live_status(&update.state);
        if update.applied {
            self.coordinator
                .ledger
                .save_run_projection(&RunProjectionRecord {
                    run_id: run_id.clone(),
                    host_epoch,
                    projection: snapshot_projection(&update.state),
                })?;
        }
        Ok(RunLiveStatus {
            run_id,
            host_epoch,
            status,
        })
    }

    /// Reads the latest durable state for a run without changing ownership or process state.
    ///
    /// # Errors
    /// Returns an error when the projection cannot be read.
    pub fn live_status(&self, run_id: &str) -> Result<Option<RunLiveStatus>, RuntimeError> {
        let record = self.coordinator.ledger.find_run_projection(run_id)?;
        Ok(record.map(|record| RunLiveStatus {
            run_id: record.run_id,
            host_epoch: record.host_epoch,
            status: projected_live_status(&restore_projection(&record.projection)),
        }))
    }

    fn require_owner(
        &self,
        run_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        let ingress = self.coordinator.ledger.host_ingress()?;
        if ingress.epoch != host_epoch {
            return Err(RuntimeError::Ledger(gent_ports::LedgerError::StaleEpoch {
                command: host_epoch,
                active: ingress.epoch,
            }));
        }
        if ingress.mode == IngressMode::Closed {
            return Err(RuntimeError::Ledger(
                gent_ports::LedgerError::IngressClosed { epoch: host_epoch },
            ));
        }
        let owned = self
            .coordinator
            .ledger
            .find_run_lease(run_id)?
            .is_some_and(|lease| {
                lease.coordinator_id == coordinator_id && lease.host_epoch == host_epoch
            });
        if !owned {
            return Err(invariant("projection reporter does not own the run"));
        }
        if self
            .coordinator
            .ledger
            .find_run_session_binding(run_id)?
            .is_none()
        {
            return Err(invariant(
                "run has no daemon-owned provider session binding",
            ));
        }
        Ok(())
    }
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}
