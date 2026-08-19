//! Stable value types shared by every public Gent crate.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
mod agent_chat;
mod agent_chat_compaction;
mod agent_chat_intent;
mod agent_chat_ledger;
mod agent_chat_prompt;
mod agent_chat_run_context;
mod agent_chat_switch;
mod agent_chat_terminal_settlement;
mod attachments;
#[cfg(test)]
mod contract_edge_tests;
mod conversation_activity;
mod conversation_artifact;
mod conversation_content;
mod conversation_context;
mod conversation_prompts;
mod conversations;
mod decision;
mod doctor;
mod event_page;
mod external_provider_bridge;
mod git_operations;
mod goal;
mod lifecycle_signal;
mod lifecycle_state;
mod mcp_connectors;
mod normalized_session;
mod observer_tap;
mod onboarding;
mod orchestration;
mod orchestration_facts;
#[cfg(test)]
mod orchestration_tests;
mod permission_control;
mod policies;
mod provider_auth;
mod provider_lifecycle_values;
mod reviewed_plan;
mod run_checkpoints;
mod run_lifecycle_fact;
mod runtime_maintenance;
mod runtime_update;
mod sandbox_launch;
#[cfg(test)]
mod sandbox_launch_tests;
mod tool_activity;
mod tool_sources;
mod turn_follow;
mod workspaces;
pub use agent_chat::{
    AgentChatConversationDetail, AgentChatConversationSummary, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRun, AgentChatRunState, AgentChatSelection,
    AgentChatSelectionError, NormalizedTranscriptAppend, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, NormalizedTranscriptPage,
};
pub use agent_chat_compaction::{AgentChatCompactionFact, AgentChatCompactionFailure};
pub use agent_chat_intent::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatDecisionResponse, AgentChatRequestId,
    AgentChatRunId,
};
pub use agent_chat_ledger::{AgentChatConversationCreate, AgentChatConversationCreated};
pub use agent_chat_prompt::{
    AgentChatPromptCreate, AgentChatPromptDelivery, AgentChatPromptDisposition,
    AgentChatPromptSaved,
};
pub use agent_chat_run_context::{AgentChatRunContext, AgentChatRunContextOrigin};
pub use agent_chat_switch::{AgentChatSelectionSwitch, AgentChatSelectionSwitched};
pub use agent_chat_terminal_settlement::AgentChatTerminalSettlement;
pub use attachments::{
    AttachmentMetadata, AttachmentOperation, AttachmentState, AttachmentTransfer, TurnAttachment,
};
pub use conversation_activity::{
    ActivityWorkKind, CONVERSATION_ACTIVITY_SCHEMA_VERSION, ConversationActivityFact,
    ConversationActivityPage, ConversationActivityScope,
};
pub use conversation_artifact::{
    ConversationArtifact, ConversationArtifactKind, ConversationArtifactStatus,
};
pub use conversation_content::{
    ConversationContentCursor, ConversationContentCursorError, ConversationContentEntry,
    ConversationContentPage,
};
pub use conversation_context::FrozenConversationContext;
pub use conversation_prompts::{ConversationMessage, ConversationPrompt};
pub use conversations::{
    ConversationArtifactSummary, ConversationListItem, ConversationRecord, ConversationRunStatus,
    ConversationStatus, ConversationTimeline, ConversationTimelineRun, DurableTurnPhase,
    TurnRecord,
};
pub use decision::{DecisionCommand, DecisionSettlement, DecisionSettlementPhase};
pub use doctor::{
    CompatibilityTrust, DoctorNextAction, ExecutableIdentity, McpDoctorStatus, McpPermissionStatus,
    PrivateBridgeAvailability, PublicProviderStatus,
};
pub use event_page::EventPage;
pub use external_provider_bridge::{ExternalProviderSession, ExternalProviderTerminal};
pub use git_operations::{GitOperationKind, GitOperationPhase, GitOperationRecord};
pub use goal::{
    GOAL_SCHEMA_VERSION, GoalBinding, GoalContractError, GoalProjection, GoalRecord, GoalStatus,
    GoalTransition,
};
pub use lifecycle_signal::NormalizedLifecycleSignal;
pub use lifecycle_state::{ConversationLiveStatus, RootActivity, TurnPhase, WorkPhase};
pub use mcp_connectors::{McpConnectorPhase, McpConnectorRecord};
pub use normalized_session::{
    NormalizedSessionBatch, NormalizedSessionBatchResult, NormalizedSessionLifecycle,
};
pub use observer_tap::{LegacyLifecycleTap, ObserverDiagnostic, ObserverDiagnosticCode};
pub use onboarding::{OnboardingBranch, OnboardingProvider, OnboardingReadiness, OnboardingState};
pub use orchestration::*;
pub use orchestration_facts::{TaskGraphFact, TaskGraphFactKind, TaskGraphFactPage};
pub use permission_control::*;
pub use policies::{
    PermissionCategory, PermissionMode, PermissionRequest, PolicyRecord, PolicyScope,
    SandboxEnforcement,
};
pub use provider_auth::{
    ProviderAuthBinaryLock, ProviderAuthChallenge, ProviderAuthContractError,
    ProviderAuthLifecycle, ProviderAuthMethod, ProviderAuthMethodSelection, ProviderAuthProvider,
    ProviderAuthStatus,
};
pub use provider_lifecycle_values::{NormalizedProviderEvent, ProviderEvent, RunVersionLock};
pub use reviewed_plan::{
    ContextPolicy, PlanAction, PlanActionKind, PlanArtifact, PlanDiff, PlanDiffKind,
    PlanPermissionPreview, PlanRevision, PlanRisk, PlanRiskKind, PlanRiskSeverity, PlanStatus,
    ReviewedPlanContractError, ReviewedPlanId, StartImplementationRequest,
    StartImplementationResult,
};
pub use run_checkpoints::RunCheckpointRecord;
pub use run_lifecycle_fact::{RunLifecycleFact, RunLifecycleFactPage, RunLiveStatus};
pub use runtime_maintenance::{RuntimeMaintenanceReport, RuntimeMaintenanceRequest};
pub use runtime_update::{
    RUNTIME_RELEASE_INDEX_VERSION, RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact,
    RuntimeReleaseChannel, RuntimeReleaseIdentity, RuntimeReleaseIndex, RuntimeReleaseManifest,
    RuntimeReleaseOffer, RuntimeStagingReceipt, RuntimeUpdateCandidate, RuntimeUpdateCheckReport,
    RuntimeUpdateCheckRequest, RuntimeUpdateCheckState, RuntimeUpdateFailure, RuntimeUpdateHandoff,
    RuntimeUpdateRecord, RuntimeUpdateStage, RuntimeUpdateStatus, RuntimeVersion,
    SignedRuntimeRelease, SignedRuntimeReleaseIndex,
};
pub use sandbox_launch::{
    SandboxBackendId, SandboxLaunchAttestation, SandboxLaunchContractError, SandboxLaunchProfile,
    SandboxNetworkPolicy, SandboxResourceLimits, SandboxedLaunchRequest,
};
pub use tool_activity::{ToolActivity, ToolCategory, ToolPhase};
pub use tool_sources::{ToolSourceKind, ToolSourceRecord};
pub use turn_follow::TurnTerminal;
pub use workspaces::{RepositoryRecord, WorkspaceRecord, WorktreeRecord};
pub const PROTOCOL_MIN: u16 = 1;
pub const PROTOCOL_MAX: u16 = 1;
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct HostEpoch(pub u64);
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ReceiptId(pub String);
impl ReceiptId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}
impl Default for ReceiptId {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceiptStatus {
    Accepted,
    Settled,
    Unprovable,
    Rejected,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub status: ReceiptStatus,
    pub host_epoch: HostEpoch,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub cursor: u64,
    pub event_id: String,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub kind: String,
    pub payload: Value,
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilitySet(pub Vec<String>);
impl CapabilitySet {
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Self {
        let mut shared = self
            .0
            .iter()
            .filter(|capability| other.0.contains(*capability))
            .cloned()
            .collect::<Vec<_>>();
        shared.sort();
        shared.dedup();
        Self(shared)
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub host_epoch: HostEpoch,
    pub protocol_min: u16,
    pub protocol_max: u16,
    pub capabilities: CapabilitySet,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyStatus {
    pub name: String,
    pub present: bool,
    pub version: Option<String>,
    pub remediation: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DoctorReport {
    pub dependencies: Vec<DependencyStatus>,
    pub public_providers: Vec<PublicProviderStatus>,
    pub mcp: McpDoctorStatus,
    pub private_bridge: PrivateBridgeAvailability,
    pub next_action: DoctorNextAction,
}

#[cfg(test)]
mod tests {
    use super::CapabilitySet;

    #[test]
    fn capability_intersection_is_sorted_and_unique() {
        let left = CapabilitySet(vec!["events".into(), "receipts".into(), "events".into()]);
        let right = CapabilitySet(vec!["events".into(), "status".into()]);
        assert_eq!(
            left.intersection(&right),
            CapabilitySet(vec!["events".into()])
        );
    }
}
