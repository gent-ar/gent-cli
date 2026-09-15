use gent_types::{
    Command, DecisionCommand, DecisionSettlement, DecisionSettlementPhase, Event, EventPage,
    HostEpoch, Receipt, ReceiptStatus, RunVersionLock,
};

use crate::{HostIngress, RunSessionBinding};

#[derive(Clone, Debug, PartialEq)]
pub enum ReceiptClaim {
    Existing(Receipt),
    Accepted(Receipt),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionClaim {
    Created(DecisionSettlement),
    Existing(DecisionSettlement),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionPhaseUpdate {
    Applied(DecisionSettlement),
    Current(DecisionSettlement),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    pub run_id: String,
    pub parent_run_id: Option<String>,
    pub provider: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeLease {
    pub worktree_id: String,
    pub run_id: String,
    pub lease_token: String,
    pub host_epoch: HostEpoch,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunLease {
    pub run_id: String,
    pub coordinator_id: String,
    pub host_epoch: HostEpoch,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunLeaseClaim {
    Acquired(RunLease),
    Contended(RunLease),
    Recovered {
        previous: RunLease,
        current: RunLease,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaseClaim {
    Acquired(WorktreeLease),
    Contended(WorktreeLease),
    Recovered {
        previous: WorktreeLease,
        current: WorktreeLease,
    },
}
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("stale host epoch: command {command:?}, active {active:?}")]
    StaleEpoch {
        command: HostEpoch,
        active: HostEpoch,
    },
    #[error("ingress is closed at epoch {epoch:?}")]
    IngressClosed { epoch: HostEpoch },
    #[error("durable invariant violated: {0}")]
    Invariant(String),
    #[error("{0}")]
    Rejected(gent_types::AgentChatRejection),
    #[error("ledger failure: {0}")]
    Storage(String),
}
#[allow(clippy::missing_errors_doc)]
pub trait Ledger: Send + Sync {
    fn host_ingress(&self) -> Result<HostIngress, LedgerError>;
    fn close_ingress(&self, epoch: HostEpoch) -> Result<HostIngress, LedgerError>;
    fn fence_and_open(&self, epoch: HostEpoch) -> Result<HostIngress, LedgerError>;
    fn claim_command(
        &self,
        command: &Command,
        accepted: &Event,
    ) -> Result<ReceiptClaim, LedgerError>;
    fn settle_receipt(
        &self,
        idempotency_key: &str,
        status: ReceiptStatus,
        terminal: &Event,
    ) -> Result<Receipt, LedgerError>;
    fn claim_decision(&self, command: &DecisionCommand) -> Result<DecisionClaim, LedgerError>;
    fn find_decision(&self, decision_id: &str) -> Result<Option<DecisionSettlement>, LedgerError>;
    /// Advances a decision only if its durable phase still equals `expected`.
    /// # Errors
    /// Returns an error when the decision is unknown or persistence fails.
    fn replace_decision_phase(
        &self,
        decision_id: &str,
        expected: &DecisionSettlementPhase,
        next: &DecisionSettlementPhase,
    ) -> Result<DecisionPhaseUpdate, LedgerError>;
    /// Appends a cursor-ordered event outside the command receipt transaction.
    /// # Errors
    /// Returns an error when the event cannot be persisted.
    fn append_event(&self, event: &Event) -> Result<Event, LedgerError>;
    /// Finds one durable event by its producer-stable identity.
    /// # Errors
    /// Returns an error when the event cannot be read.
    fn find_event(&self, event_id: &str) -> Result<Option<Event>, LedgerError> {
        let _ = event_id;
        Err(LedgerError::Invariant("event lookup is unavailable".into()))
    }
    /// Reads one bounded page from the immutable cursor-ordered event log.
    /// # Errors
    /// Returns an error when events cannot be read.
    fn read_event_page(&self, after_cursor: u64, limit: usize) -> Result<EventPage, LedgerError>;
    /// Creates an immutable lineage node. A child must name an existing parent.
    ///
    /// # Errors
    /// Returns an error when lineage invariants or persistence fail.
    fn create_run(&self, run: &RunRecord) -> Result<(), LedgerError>;
    /// Atomically persists a new run, immutable executable lock, and coordinator lease.
    ///
    /// # Errors
    /// Returns an error when ingress is closed or stale, lineage or provider identities differ,
    /// or any durable reservation step fails.
    fn reserve_run_start(
        &self,
        run: &RunRecord,
        lock: &RunVersionLock,
        lease: &RunLease,
    ) -> Result<(), LedgerError>;
    /// Atomically locks and leases a pre-created run; errors when provider or epoch differs.
    fn activate_existing_run_start(
        &self,
        lock: &RunVersionLock,
        lease: &RunLease,
    ) -> Result<RunLeaseClaim, LedgerError> {
        let _ = (lock, lease);
        Err(LedgerError::Invariant(
            "ledger does not support activation of an existing run".into(),
        ))
    }
    /// Reads one lineage node; errors when the run cannot be read.
    fn find_run(&self, run_id: &str) -> Result<Option<RunRecord>, LedgerError>;
    /// Persists the immutable executable identity attributed to a run.
    /// # Errors: Returns an error if the run does not exist, already has a lock, or persistence fails.
    fn save_run_version_lock(&self, run_id: &str, lock: &RunVersionLock)
    -> Result<(), LedgerError>;
    /// Reads the immutable executable identity attributed to a run.
    /// # Errors: Returns an error when the lock cannot be read.
    fn find_run_version_lock(&self, run_id: &str) -> Result<Option<RunVersionLock>, LedgerError>;
    /// Persists a provider-native session reported by the daemon for a durable run.
    /// It is idempotent only when identical; a conflicting binding is rejected.
    /// # Errors
    /// Returns an error when binding persistence is unsupported or fails.
    fn save_run_session_binding(&self, binding: &RunSessionBinding) -> Result<(), LedgerError> {
        let _ = binding;
        Err(LedgerError::Invariant(
            "ledger does not support provider session bindings".into(),
        ))
    }
    fn retire_run_session_binding(
        &self,
        binding: &RunSessionBinding,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        let _ = (binding, host_epoch);
        Err(LedgerError::Invariant(
            "ledger does not support provider session bindings".into(),
        ))
    }
    /// Reads the daemon-owned provider session identity to use for resume.
    /// # Errors
    /// Returns an error when the binding cannot be read.
    fn find_run_session_binding(
        &self,
        run_id: &str,
    ) -> Result<Option<RunSessionBinding>, LedgerError> {
        let _ = run_id;
        Ok(None)
    }
    /// # Errors
    /// Returns an error when the requesting epoch or run is invalid, or persistence fails.
    fn claim_run_lease(&self, requested: &RunLease) -> Result<RunLeaseClaim, LedgerError>;
    /// Reads the coordinator currently holding a run, if any.
    ///
    /// # Errors
    /// Returns an error when the lease cannot be read.
    fn find_run_lease(&self, run_id: &str) -> Result<Option<RunLease>, LedgerError>;
    /// Atomically obtains a worktree lease or reports its durable owner.
    ///
    /// # Errors
    /// Returns an error when the requesting epoch or run is invalid, or persistence fails.
    fn claim_worktree_lease(&self, requested: &WorktreeLease) -> Result<LeaseClaim, LedgerError>;
    /// Reads the current lease, if any.
    ///
    /// # Errors
    /// Returns an error when the lease cannot be read.
    fn find_worktree_lease(&self, worktree_id: &str) -> Result<Option<WorktreeLease>, LedgerError>;
}
