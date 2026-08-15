//! Coordinator orchestration over pure policy and durable ports.
mod attachment_receipts;
mod attachments;
mod automation_executions;
pub mod catalog;
mod conversations;
mod decisions;
mod dependency_actions;
mod events;
mod git_operations;
mod legacy_observer;
mod policies;
mod provider_lifecycle;
mod public_runs;
mod run_checkpoints;
mod run_projections;
mod tool_sources;
mod workspaces;
pub use attachments::AttachmentService;
pub use dependency_actions::DependencyActionService;
use gent_core::{Run, switch_provider};
use gent_ports::{
    HostIngress, LeaseClaim, Ledger, LedgerError, ReceiptClaim, RunLease, RunLeaseClaim, RunRecord,
    WorktreeLease,
};
use gent_types::{
    CapabilitySet, Command, Event, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN, Receipt, ReceiptStatus,
    RunVersionLock,
};
pub use legacy_observer::{LegacyObserver, ObserverPoll};
pub use provider_lifecycle::{ProviderLifecycleEffect, ProviderLifecycleIngress};
pub use public_runs::{ProviderRunAuthority, PublicRunService};
pub use run_projections::RunProjectionService;
#[derive(Clone, Debug)]
pub struct Coordinator<L> {
    ledger: L,
    capabilities: CapabilitySet,
}
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error("unknown decision: {0}")]
    UnknownDecision(String),
    #[error("decision was changed by another coordinator too often")]
    DecisionContention,
    #[error(transparent)]
    ProviderLock(#[from] gent_drivers::lock::LockError),
    #[error(transparent)]
    ProviderRun(#[from] gent_ports::PublicProviderRunError),
    #[error(transparent)]
    Core(#[from] gent_core::CoreError),
    #[error(transparent)]
    Port(#[from] gent_ports::PortError),
}
impl<L: Ledger> Coordinator<L> {
    #[must_use]
    pub fn new(ledger: L, capabilities: CapabilitySet) -> Self {
        Self {
            ledger,
            capabilities,
        }
    }
    /// Returns the negotiated host state.
    /// # Errors
    /// Returns an error when the durable host state cannot be read.
    pub fn status(&self) -> Result<HostStatus, RuntimeError> {
        Ok(HostStatus {
            host_epoch: self.ledger.host_ingress()?.epoch,
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: self.capabilities.clone(),
        })
    }
    /// Atomically accepts one command, or returns the already durable receipt for its key.
    /// # Errors
    /// Returns an error when the host fence rejects ingress or durable persistence fails.
    #[allow(clippy::needless_pass_by_value)] // The coordinator owns the wire command boundary.
    pub fn submit(&self, command: &Command) -> Result<Receipt, RuntimeError> {
        let accepted = Event {
            cursor: 0,
            event_id: format!("{}:accepted", command.receipt_id.0),
            receipt_id: command.receipt_id.clone(),
            host_epoch: command.host_epoch,
            kind: "commandAccepted".into(),
            payload: command.payload.clone(),
        };
        // A concurrent retry can observe the short accepted-to-terminal window. Both callers
        // then settle the same receipt; the ledger atomically lets one append the terminal event
        // and returns that terminal receipt to the other.
        let receipt = match self.ledger.claim_command(command, &accepted)? {
            ReceiptClaim::Existing(receipt) | ReceiptClaim::Accepted(receipt) => receipt,
        };
        let status = terminal_status(&command.kind);
        let terminal = Event {
            cursor: 0,
            event_id: format!("{}:terminal", receipt.receipt_id.0),
            receipt_id: receipt.receipt_id.clone(),
            host_epoch: receipt.host_epoch,
            kind: terminal_kind(&status).into(),
            payload: serde_json::json!({ "status": status }),
        };
        Ok(self
            .ledger
            .settle_receipt(&receipt.idempotency_key, status, &terminal)?)
    }
    /// Closes mutation ingress as the first half of an authority transfer.
    /// # Errors
    /// Returns an error when the caller no longer owns the active epoch.
    pub fn close_ingress(&self, epoch: gent_types::HostEpoch) -> Result<HostIngress, RuntimeError> {
        Ok(self.ledger.close_ingress(epoch)?)
    }
    /// Fences the old writer and opens the successor epoch in one durable operation.
    ///
    /// # Errors
    /// Returns an error when ingress was not closed or the caller has been superseded.
    pub fn fence_and_open(
        &self,
        epoch: gent_types::HostEpoch,
    ) -> Result<HostIngress, RuntimeError> {
        Ok(self.ledger.fence_and_open(epoch)?)
    }
    /// Persists an immutable root run.
    ///
    /// # Errors
    /// Returns an error when the run already exists or persistence fails.
    #[allow(clippy::needless_pass_by_value)] // The coordinator owns the root-run handoff boundary.
    pub fn create_run(&self, run: Run) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_run(&to_record(&run))?)
    }
    /// Persists the immutable executable identity to be rechecked before run resume.
    ///
    /// # Errors
    /// Returns an error when the run is unknown, already locked, or persistence fails.
    pub fn lock_run_version(
        &self,
        run_id: &str,
        lock: &RunVersionLock,
    ) -> Result<(), RuntimeError> {
        Ok(self.ledger.save_run_version_lock(run_id, lock)?)
    }
    /// Persists a provider-switch child instead of mutating the source run.
    ///
    /// # Errors
    /// Returns an error when lineage persistence fails.
    pub fn switch_provider(
        &self,
        run: &Run,
        child_id: String,
        provider: String,
    ) -> Result<Run, RuntimeError> {
        let child = switch_provider(run, child_id, provider);
        self.ledger.create_run(&to_record(&child))?;
        Ok(child)
    }
    /// Atomically claims the coordinator role for a durable run.
    ///
    /// # Errors
    /// Returns an error when the run is unknown, the epoch is stale, or persistence fails.
    pub fn claim_run_lease(&self, lease: &RunLease) -> Result<RunLeaseClaim, RuntimeError> {
        Ok(self.ledger.claim_run_lease(lease)?)
    }

    /// Atomically acquires, reports contention, or recovers a stale worktree lease.
    ///
    /// # Errors
    /// Returns an error when the request has a stale epoch or its run is unknown.
    #[allow(clippy::needless_pass_by_value)] // The coordinator owns the lease handoff boundary.
    pub fn claim_worktree_lease(&self, lease: &WorktreeLease) -> Result<LeaseClaim, RuntimeError> {
        Ok(self.ledger.claim_worktree_lease(lease)?)
    }
}
fn terminal_status(kind: &str) -> ReceiptStatus {
    if kind == "decision" {
        ReceiptStatus::Unprovable
    } else {
        ReceiptStatus::Settled
    }
}
fn terminal_kind(status: &ReceiptStatus) -> &'static str {
    if *status == ReceiptStatus::Unprovable {
        "decisionUnprovable"
    } else {
        "commandSettled"
    }
}
pub(crate) fn to_record(run: &Run) -> RunRecord {
    RunRecord {
        run_id: run.id.clone(),
        parent_run_id: run.parent_run_id.clone(),
        provider: run.provider.clone(),
    }
}

#[cfg(test)]
mod tests {
    use gent_ports::{IngressMode, Ledger};
    use gent_store::SqliteLedger;
    use gent_types::{CapabilitySet, Command, HostEpoch, ReceiptId};
    use serde_json::json;

    use super::*;

    fn command(key: &str, epoch: u64, kind: &str) -> Command {
        Command {
            receipt_id: ReceiptId::new(),
            idempotency_key: key.into(),
            host_epoch: HostEpoch(epoch),
            kind: kind.into(),
            payload: json!({ "example": true }),
        }
    }

    #[test]
    fn acceptance_and_terminal_events_are_idempotent() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
        let command = command("once", 1, "ping");
        let first = coordinator.submit(&command).unwrap();
        assert_eq!(first, coordinator.submit(&command).unwrap());
        assert!(matches!(
            ledger.resume_events(0).unwrap(),
            gent_types::EventResume::Delta { events } if events.len() == 2
        ));
    }

    #[test]
    fn a_closed_or_fenced_host_cannot_accept_commands() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        assert_eq!(
            coordinator.close_ingress(HostEpoch(1)).unwrap().mode,
            IngressMode::Closed
        );
        assert!(matches!(
            coordinator.submit(&command("closed", 1, "ping")),
            Err(RuntimeError::Ledger(LedgerError::IngressClosed { .. }))
        ));
        assert_eq!(
            coordinator.fence_and_open(HostEpoch(1)).unwrap().epoch,
            HostEpoch(2)
        );
        assert!(matches!(
            coordinator.submit(&command("old", 1, "ping")),
            Err(RuntimeError::Ledger(LedgerError::StaleEpoch { .. }))
        ));
    }

    #[test]
    fn decisions_settle_without_provider_acknowledgement() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        assert_eq!(
            coordinator
                .submit(&command("decision", 1, "decision"))
                .unwrap()
                .status,
            ReceiptStatus::Unprovable
        );
    }

    #[test]
    fn provider_switches_and_stale_leases_are_durable() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        let root = Run {
            id: "root".into(),
            parent_run_id: None,
            provider: "claude".into(),
        };
        coordinator.create_run(root.clone()).unwrap();
        let child = coordinator
            .switch_provider(&root, "child".into(), "codex".into())
            .unwrap();
        let first = WorktreeLease {
            worktree_id: "tree".into(),
            run_id: root.id,
            lease_token: "one".into(),
            host_epoch: HostEpoch(1),
        };
        assert!(matches!(
            coordinator.claim_worktree_lease(&first),
            Ok(LeaseClaim::Acquired(_))
        ));
        coordinator.close_ingress(HostEpoch(1)).unwrap();
        coordinator.fence_and_open(HostEpoch(1)).unwrap();
        let second = WorktreeLease {
            worktree_id: "tree".into(),
            run_id: child.id,
            lease_token: "two".into(),
            host_epoch: HostEpoch(2),
        };
        assert!(matches!(
            coordinator.claim_worktree_lease(&second),
            Ok(LeaseClaim::Recovered { .. })
        ));
    }
}
