//! Ports implemented by infrastructure or private integrations.
use async_trait::async_trait;
use gent_types::{
    Command, DecisionCommand, DecisionSettlement, DecisionSettlementPhase, Event, EventResume,
    EventSnapshot, HostEpoch, Receipt, ReceiptStatus, RunVersionLock,
};
mod automation_execution_ledger;
mod capability_catalog;
mod conversation_artifacts;
mod conversation_ledger;
mod external_provider_bridge;
mod git_operation_ledger;
mod policy_ledger;
mod run_checkpoint_ledger;
mod run_projections;
mod run_sessions;
mod tool_source_ledger;
mod workspace_ledger;
pub use automation_execution_ledger::{AutomationExecutionLedger, AutomationExecutionUpdate};
pub use capability_catalog::CapabilityCatalogLedger;
pub use conversation_artifacts::ConversationArtifactLedger;
pub use conversation_ledger::{ConversationLedger, TurnPhaseUpdate};
pub use external_provider_bridge::{
    ExternalProviderBridge, ExternalProviderSession, ExternalProviderTerminal,
};
pub use git_operation_ledger::{GitOperationLedger, GitOperationUpdate};
pub use policy_ledger::PolicyLedger;
pub use run_checkpoint_ledger::RunCheckpointLedger;
pub use run_projections::RunProjectionLedger;
pub use run_sessions::RunSessionBinding;
pub use tool_source_ledger::ToolSourceLedger;
pub use workspace_ledger::WorkspaceLedger;
#[derive(Debug, thiserror::Error)]
pub enum PortError {
    #[error("provider bridge failure: {0}")]
    Provider(String),
    #[error("provider bridge operation is unavailable: {0}")]
    Unavailable(String),
}
/// Expected failures from an owned public-provider lifecycle operation.
#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum PublicProviderRunError {
    #[error("provider executable changed before spawn or resume")]
    ProviderChanged,
    #[error("provider run is not active")]
    NotActive,
    #[error("provider lifecycle failed: {0}")]
    Failed(String),
}
#[async_trait]
pub trait ProviderDriver: Send + Sync {
    async fn submit(&self, command: Command) -> Result<(), PortError>;
}
/// Daemon-owned public provider lifecycle boundary.
///
/// Implementations may only receive locks derived from Claude or Codex. Private bridges are
/// represented separately by [`ExternalProviderBridge`] and cannot enter this lifecycle.
pub trait PublicProviderRunner: Send + Sync {
    /// Starts a new process only after the caller has durably reserved its run.
    /// # Errors
    /// Returns a changed-binary or launcher failure without silently substituting a provider.
    fn start(&self, run_id: &str, lock: &RunVersionLock) -> Result<(), PublicProviderRunError>;
    /// Resumes a process only after the caller has re-established durable ownership.
    /// # Errors
    /// Returns a changed-binary or launcher failure without silently substituting a provider.
    fn resume(
        &self,
        run_id: &str,
        lock: &RunVersionLock,
        session_id: &str,
    ) -> Result<(), PublicProviderRunError>;
    /// Interrupts the complete process tree currently owned by `run_id`.
    /// # Errors
    /// Returns an error when no process is active or process-tree interruption fails.
    fn interrupt(&self, run_id: &str) -> Result<(), PublicProviderRunError>;
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
/// Result of atomically creating or locating a decision by either stable identifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionClaim {
    Created(DecisionSettlement),
    Existing(DecisionSettlement),
}
/// Result of an optimistic phase update. `Current` preserves a concurrent writer's state.
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
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn host_ingress(&self) -> Result<HostIngress, LedgerError>;
    /// Closes ingress if and only if `epoch` is still authoritative.
    /// # Errors
    /// Returns an error when the epoch is stale or durable state cannot be updated.
    fn close_ingress(&self, epoch: HostEpoch) -> Result<HostIngress, LedgerError>;
    /// Atomically creates the next host epoch and opens it for its new writer.
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
    /// Atomically creates a pending decision or returns the record owning either identifier.
    ///
    /// # Errors
    /// Returns an error when the decision cannot be persisted or read.
    fn claim_decision(&self, command: &DecisionCommand) -> Result<DecisionClaim, LedgerError>;
    /// Reads one durable decision settlement state.
    ///
    /// # Errors
    /// Returns an error when the decision cannot be read.
    fn find_decision(&self, decision_id: &str) -> Result<Option<DecisionSettlement>, LedgerError>;
    /// Advances a decision only if its durable phase still equals `expected`.
    ///
    /// # Errors
    /// Returns an error when the decision is unknown or persistence fails.
    fn replace_decision_phase(
        &self,
        decision_id: &str,
        expected: &DecisionSettlementPhase,
        next: &DecisionSettlementPhase,
    ) -> Result<DecisionPhaseUpdate, LedgerError>;
    /// Appends a cursor-ordered event outside the command receipt transaction.
    ///
    /// # Errors
    /// Returns an error when the event cannot be persisted.
    fn append_event(&self, event: &Event) -> Result<Event, LedgerError>;
    /// Safely resumes an event feed, returning a replacement snapshot for stale cursors.
    ///
    /// # Errors
    /// Returns an error when events cannot be read.
    fn resume_events(&self, cursor: u64) -> Result<EventResume, LedgerError>;
    /// Atomically persists a newer projection snapshot and retires its covered event prefix.
    ///
    /// # Errors
    /// Returns an error if the snapshot regresses, exceeds the durable event head, or cannot commit.
    fn compact_events(&self, snapshot: &EventSnapshot) -> Result<(), LedgerError>;
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
    /// Persists a provider-native session reported by the daemon for a durable run.
    /// It is idempotent only when identical; a conflicting binding is rejected.
    ///
    /// # Errors
    /// Returns an error when binding persistence is unsupported or fails.
    fn save_run_session_binding(&self, binding: &RunSessionBinding) -> Result<(), LedgerError> {
        let _ = binding;
        Err(LedgerError::Invariant(
            "ledger does not support provider session bindings".into(),
        ))
    }
    /// Reads the daemon-owned provider session identity to use for resume.
    ///
    /// # Errors
    /// Returns an error when the binding cannot be read.
    fn find_run_session_binding(
        &self,
        run_id: &str,
    ) -> Result<Option<RunSessionBinding>, LedgerError> {
        let _ = run_id;
        Ok(None)
    }
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
