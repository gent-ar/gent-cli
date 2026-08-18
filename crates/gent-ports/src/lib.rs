use gent_types::{
    Command, DecisionCommand, DecisionSettlement, DecisionSettlementPhase, Event, EventResume,
    EventSnapshot, HostEpoch, Receipt, ReceiptStatus, RunVersionLock,
};
mod agent_chat_ledger;
mod attachment_blobs;
mod attachment_ledger;
mod capability_catalog;
mod conversation_activity_ledger;
mod conversation_artifacts;
mod conversation_content;
mod conversation_ledger;
mod conversation_prompt_ledger;
mod dependency_action_executor;
mod external_provider_bridge;
mod git_executor;
mod git_operation_ledger;
mod goal_ledger;
mod ingress;
mod legacy_event_tap;
mod mcp_connector_executor;
mod mcp_connector_ledger;
mod package_install;
mod policy_ledger;
mod private_claurst_bridge;
mod provider_auth_discovery;
mod provider_driver;
mod public_provider_resolver;
mod public_provider_runner;
mod reviewed_plan_ledger;
mod run_checkpoint_ledger;
mod run_projections;
mod run_sessions;
mod run_version_authorizer;
pub mod runtime_update;
mod tool_source_ledger;
mod transcript_ledger;
mod workspace_ledger;
pub use agent_chat_ledger::{
    AgentChatLedger, AgentChatPromptDispatchLedger, AgentChatPromptLedger, AgentChatReadLedger,
    AgentChatSelectionLedger,
};
pub use attachment_blobs::AttachmentBlobStore;
pub use attachment_ledger::{AttachmentClaim, AttachmentLedger};
pub use capability_catalog::CapabilityCatalogLedger;
pub use conversation_activity_ledger::*;
pub use conversation_artifacts::ConversationArtifactLedger;
pub use conversation_content::ConversationContentReader;
pub use conversation_ledger::{ConversationLedger, TurnPhaseUpdate};
pub use conversation_prompt_ledger::{ConversationPromptLedger, ConversationPromptSave};
pub use dependency_action_executor::{
    DependencyActionExecutor, DependencyActionExecutorError, DependencyActionOperation,
};
pub use external_provider_bridge::ExternalProviderBridge;
pub use gent_types::{ExternalProviderSession, ExternalProviderTerminal};
pub use git_executor::{GitExecutor, GitExecutorError, GitStatusOperation, GitStatusSummary};
pub use git_operation_ledger::{GitOperationLedger, GitOperationUpdate};
pub use goal_ledger::*;
pub use ingress::{HostIngress, IngressMode};
pub use legacy_event_tap::LegacyEventTap;
pub use mcp_connector_executor::{
    McpConnectOperation, McpConnectionSummary, McpConnectorError, McpConnectorExecutor,
};
pub use mcp_connector_ledger::{
    McpConnectorLease, McpConnectorLeaseClaim, McpConnectorLedger, McpConnectorUpdate,
};
pub use package_install::{
    ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError,
};
pub use policy_ledger::PolicyLedger;
pub use private_claurst_bridge::{
    ClaurstCheckpoint, ClaurstDrainBatch, ClaurstDrainRequest, ClaurstFactValue,
    ClaurstFailureClassification, ClaurstGoalProjection, ClaurstNormalizedFact,
    ClaurstSessionBinding, ClaurstSourceId, ClaurstTerminal, MAX_PRIVATE_CLAURST_DRAIN_FACTS,
    PrivateClaurstBridge,
};
pub use provider_auth_discovery::{
    ProviderAuthAuthentication, ProviderAuthDiscovery, ProviderAuthDiscoveryError,
    ProviderAuthDiscoveryPort,
};
pub use provider_driver::ProviderDriver;
pub use public_provider_resolver::PublicProviderResolver;
pub use public_provider_runner::{PublicProviderRunError, PublicProviderRunner};
pub use reviewed_plan_ledger::ReviewedPlanLedger;
pub use run_checkpoint_ledger::RunCheckpointLedger;
pub use run_projections::RunProjectionLedger;
pub use run_sessions::RunSessionBinding;
pub use run_version_authorizer::RunVersionAuthorizer;
pub use tool_source_ledger::ToolSourceLedger;
pub use transcript_ledger::TranscriptLedger;
pub use workspace_ledger::WorkspaceLedger;
#[derive(Debug, thiserror::Error)]
pub enum PortError {
    #[error("provider bridge failure: {0}")]
    Provider(String),
    #[error("provider bridge operation is unavailable: {0}")]
    Unavailable(String),
}
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
    /// Atomically creates a pending decision or returns the record owning either identifier.
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
    /// Safely resumes an event feed, returning a replacement snapshot for stale cursors.
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
    /// Atomically locks and leases a run that was durably created before provider activation.
    /// # Errors
    /// Returns an error when the existing run/provider differs, the lock would change, or the
    /// current epoch cannot grant the requested lease.
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
    /// Reads one lineage node.
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
