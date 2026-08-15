//! Ports implemented by infrastructure or private integrations.

use async_trait::async_trait;
use gent_types::{
    Command, Event, HostEpoch, ProviderEvent, Receipt, ReceiptStatus, RunVersionLock,
};

#[derive(Debug, thiserror::Error)]
pub enum PortError {
    #[error("provider bridge failure: {0}")]
    Provider(String),
}

/// Private Claurst implementations receive only opaque references through this port.
#[async_trait]
pub trait ExternalProviderBridge: Send + Sync {
    async fn submit(&self, opaque_session: &str, command: Command) -> Result<(), PortError>;
    async fn next_event(&self, opaque_session: &str) -> Result<Option<ProviderEvent>, PortError>;
}

#[async_trait]
pub trait ProviderDriver: Send + Sync {
    async fn submit(&self, command: Command) -> Result<(), PortError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngressMode {
    Open,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostIngress {
    pub epoch: HostEpoch,
    pub mode: IngressMode,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReceiptClaim {
    Existing(Receipt),
    Accepted(Receipt),
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

/// Durable ownership claim preventing two coordinators from driving one run.
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
    #[error("ledger failure: {0}")]
    Storage(String),
}

/// Persistence boundary used by the coordinator. Implementations own durability, not policy.
pub trait Ledger: Send + Sync {
    /// Returns the durable fence and whether it currently accepts mutation ingress.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn host_ingress(&self) -> Result<HostIngress, LedgerError>;
    /// Closes ingress if and only if `epoch` is still authoritative.
    ///
    /// # Errors
    /// Returns an error when the epoch is stale or durable state cannot be updated.
    fn close_ingress(&self, epoch: HostEpoch) -> Result<HostIngress, LedgerError>;
    /// Atomically creates the next host epoch and opens it for its new writer.
    ///
    /// # Errors
    /// Returns an error when ingress is not closed, the epoch is stale, or persistence fails.
    fn fence_and_open(&self, epoch: HostEpoch) -> Result<HostIngress, LedgerError>;
    /// Atomically checks the fence, claims an idempotency key, and appends acceptance.
    ///
    /// # Errors
    /// Returns an error when ingress is closed, the epoch is stale, or the transaction fails.
    fn claim_command(
        &self,
        command: &Command,
        accepted: &Event,
    ) -> Result<ReceiptClaim, LedgerError>;
    /// Atomically transitions one receipt to a terminal status and appends its event.
    ///
    /// # Errors
    /// Returns an error when the receipt is absent, already terminal, or persistence fails.
    fn settle_receipt(
        &self,
        idempotency_key: &str,
        status: ReceiptStatus,
        terminal: &Event,
    ) -> Result<Receipt, LedgerError>;
    /// Appends a cursor-ordered event outside the command receipt transaction.
    ///
    /// # Errors
    /// Returns an error when the event cannot be persisted.
    fn append_event(&self, event: &Event) -> Result<Event, LedgerError>;
    /// Reads all events strictly after a cursor.
    ///
    /// # Errors
    /// Returns an error when events cannot be read.
    fn events_after(&self, cursor: u64) -> Result<Vec<Event>, LedgerError>;
    /// Creates an immutable lineage node. A child must name an existing parent.
    ///
    /// # Errors
    /// Returns an error when lineage invariants or persistence fail.
    fn create_run(&self, run: &RunRecord) -> Result<(), LedgerError>;
    /// Reads one lineage node.
    ///
    /// # Errors
    /// Returns an error when the run cannot be read.
    fn find_run(&self, run_id: &str) -> Result<Option<RunRecord>, LedgerError>;
    /// Persists the immutable executable identity attributed to a run.
    ///
    /// # Errors
    /// Returns an error if the run does not exist, already has a lock, or persistence fails.
    fn save_run_version_lock(&self, run_id: &str, lock: &RunVersionLock)
    -> Result<(), LedgerError>;
    /// Reads the immutable executable identity attributed to a run.
    ///
    /// # Errors
    /// Returns an error when the lock cannot be read.
    fn find_run_version_lock(&self, run_id: &str) -> Result<Option<RunVersionLock>, LedgerError>;
    /// Atomically obtains a coordinator lease for one durable run.
    ///
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
