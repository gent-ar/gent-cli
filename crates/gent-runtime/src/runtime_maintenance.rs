//! Authority-gated read model for durable runtime-update maintenance facts.

use gent_ports::{IngressMode, Ledger, runtime_update::RuntimeUpdateJournal};
use gent_types::{RuntimeMaintenanceReport, RuntimeMaintenanceRequest};

use crate::RuntimeError;

/// Permission to expose durable update maintenance facts over an approved local transport.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeMaintenanceAuthority {
    /// Observer behavior does not read a durable update attempt.
    #[default]
    Observer,
    /// An explicit planning or recovery profile may expose its own durable facts.
    Approved,
}

/// Reads one durable update record without mutating the ledger or executing an effect.
#[derive(Clone, Debug)]
pub struct RuntimeMaintenanceService<L> {
    ledger: L,
    authority: RuntimeMaintenanceAuthority,
}

impl<L> RuntimeMaintenanceService<L> {
    /// Builds an inert observer service unless an authority profile was explicitly selected.
    #[must_use]
    pub const fn new(ledger: L, authority: RuntimeMaintenanceAuthority) -> Self {
        Self { ledger, authority }
    }
}

impl<L: Ledger + RuntimeUpdateJournal> RuntimeMaintenanceService<L> {
    /// Returns one exact durable attempt plus the host ingress state.
    ///
    /// # Errors
    /// Returns an error if authority is disabled, the attempt is unknown, or durable state fails.
    pub fn read(
        &self,
        request: &RuntimeMaintenanceRequest,
    ) -> Result<RuntimeMaintenanceReport, RuntimeError> {
        if self.authority != RuntimeMaintenanceAuthority::Approved {
            return Err(error("runtime maintenance is observer-disabled"));
        }
        if request.attempt_id.trim().is_empty() {
            return Err(error("runtime maintenance attempt id is required"));
        }
        let ingress = self.ledger.host_ingress()?;
        let record = self
            .ledger
            .find_runtime_update(&request.attempt_id)?
            .ok_or_else(|| error("runtime maintenance attempt was not found"))?;
        Ok(RuntimeMaintenanceReport {
            host_epoch: ingress.epoch,
            ingress_closed: ingress.mode == IngressMode::Closed,
            record,
        })
    }
}

fn error(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}

#[cfg(test)]
mod tests {
    use gent_ports::{Ledger, runtime_update::RuntimeUpdateJournal};
    use gent_store::SqliteLedger;
    use gent_types::{
        HostEpoch, RuntimeMaintenanceRequest, RuntimeUpdateRecord, RuntimeUpdateStatus,
    };

    use super::{RuntimeMaintenanceAuthority, RuntimeMaintenanceService};

    #[test]
    fn approved_service_reads_only_the_requested_attempt_and_ingress() {
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .save_runtime_update(&RuntimeUpdateRecord {
                attempt_id: "attempt-1".into(),
                revision: 1,
                artifact_digest_sha256: "a".repeat(64),
                status: RuntimeUpdateStatus::default(),
                handoff: Default::default(),
            })
            .unwrap();
        ledger.close_ingress(HostEpoch(1)).unwrap();
        let report = RuntimeMaintenanceService::new(ledger, RuntimeMaintenanceAuthority::Approved)
            .read(&RuntimeMaintenanceRequest {
                attempt_id: "attempt-1".into(),
            })
            .unwrap();
        assert!(report.ingress_closed);
        assert_eq!(report.host_epoch, HostEpoch(1));
    }
}
