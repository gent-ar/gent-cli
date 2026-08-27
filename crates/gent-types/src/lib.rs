//! Stable value types shared by every public Gent crate.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
mod agent_chat;
mod agent_chat_checkpoint;
mod agent_chat_compaction;
mod agent_chat_conversation_config;
mod agent_chat_fork;
mod agent_chat_intent;
mod agent_chat_ledger;
mod agent_chat_prompt;
mod agent_chat_run_context;
mod agent_chat_sessions;
mod agent_chat_side_question;
mod agent_chat_switch;
mod agent_chat_terminal_settlement;
mod attachments;
mod automations;
mod command_fingerprint;
#[cfg(test)]
mod command_fingerprint_tests;
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
mod git_operations;
mod goal;
mod lifecycle_signal;
mod lifecycle_state;
mod mcp_connectors;
mod normalized_session;
mod onboarding;
mod orchestration;
mod orchestration_facts;
#[cfg(test)]
mod orchestration_tests;
mod paths;
mod permission_control;
mod policies;
mod prompt_templates;
mod provider_auth;
mod provider_lifecycle_values;
mod provider_prompt_provision;
mod provider_prompt_readiness;
mod reviewed_plan;
mod run_checkpoints;
mod run_lifecycle_fact;
mod runtime_maintenance;
mod runtime_update;
mod sandbox_launch;
#[cfg(test)]
mod sandbox_launch_tests;
mod sandbox_policy;
mod tool_activity;
mod tool_sources;
mod turn_follow;
mod workspace_git;
mod workspaces;
pub use agent_chat::{
    AgentChatConversationDetail, AgentChatConversationSummary, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRun, AgentChatRunState, AgentChatSelection,
    AgentChatSelectionError, NormalizedTranscriptAppend, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, NormalizedTranscriptPage,
};
pub use agent_chat_checkpoint::{
    AgentChatCheckpointCapture, AgentChatCheckpointRestore, AgentChatCheckpointRestored,
    AgentChatFileCheckpoint, AgentChatFileCheckpointFile, AgentChatFileSnapshot,
};
pub use agent_chat_compaction::{AgentChatCompactionFact, AgentChatCompactionFailure};
pub use agent_chat_conversation_config::{
    AgentChatConversationConfigRecord, AgentChatConversationConfigUnsupportedField,
};
pub use agent_chat_fork::{AgentChatFork, AgentChatForked};
pub use agent_chat_intent::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatDecisionResponse, AgentChatRequestId,
    AgentChatRunId,
};
pub use agent_chat_ledger::{AgentChatConversationCreate, AgentChatConversationCreated};
pub use agent_chat_prompt::{
    AgentChatPromptCreate, AgentChatPromptDelivery, AgentChatPromptDisposition,
    AgentChatPromptError, AgentChatPromptSaved, validate_tool_source_ids,
};
pub use agent_chat_run_context::{AgentChatRunContext, AgentChatRunContextOrigin};
pub use agent_chat_sessions::{AgentChatSession, AgentChatSessionId};
pub use agent_chat_side_question::{
    AgentChatSideQuestion, AgentChatSideQuestionAsked, AgentChatSideQuestionCancel,
    AgentChatSideQuestionCancelled, AgentChatSideQuestionOutcome, AgentChatSideQuestionRecord,
    AgentChatSideQuestionStatus,
};
pub use agent_chat_switch::{AgentChatSelectionSwitch, AgentChatSelectionSwitched};
pub use agent_chat_terminal_settlement::AgentChatTerminalSettlement;
pub use attachments::{
    AttachmentMetadata, AttachmentOperation, AttachmentState, AttachmentTransfer, TurnAttachment,
};
pub use automations::{
    AutomationAction, AutomationDefinition, AutomationId, AutomationNotifications, AutomationRun,
    AutomationRunId, AutomationRunStatus, AutomationRunSummary, AutomationTrigger,
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
pub use git_operations::{GitOperationKind, GitOperationPhase, GitOperationRecord};
pub use goal::{
    GOAL_SCHEMA_VERSION, GoalBinding, GoalContractError, GoalProjection, GoalRecord, GoalStatus,
    GoalTransition,
};
pub use lifecycle_signal::NormalizedLifecycleSignal;
pub use lifecycle_state::{
    ConversationAttentionStatus, ConversationErrorStatus, ConversationLiveStatus,
    ConversationProcessingStatus, ConversationWorkStatus, RootActivity, TurnPhase, WorkPhase,
};
pub use mcp_connectors::{ForgeConnectorRecord, McpConnectorPhase, McpConnectorRecord};
pub use normalized_session::{
    NormalizedSessionBatch, NormalizedSessionBatchResult, NormalizedSessionLifecycle,
};
pub use onboarding::{OnboardingBranch, OnboardingProvider, OnboardingReadiness, OnboardingState};
pub use orchestration::*;
pub use orchestration_facts::{TaskGraphFact, TaskGraphFactKind, TaskGraphFactPage};
pub use paths::{
    default_data_dir, local_socket_path, migrate_legacy_default_data_dir, resolve_sibling_binary,
    windows_pipe_name,
};
pub use permission_control::*;
pub use policies::{
    PermissionCategory, PermissionMode, PermissionRequest, PolicyRecord, PolicyScope,
    SandboxEnforcement,
};
pub use prompt_templates::{
    PROMPT_TEMPLATE_SCHEMA_VERSION, PromptTemplateError, PromptTemplateRecord,
    PromptTemplateRender, PromptTemplateVariable,
};
pub use provider_auth::{
    ProviderAuthBinaryLock, ProviderAuthChallenge, ProviderAuthContractError,
    ProviderAuthLifecycle, ProviderAuthMethod, ProviderAuthMethodSelection, ProviderAuthProvider,
    ProviderAuthStatus,
};
pub use provider_lifecycle_values::{
    NormalizedProviderEvent, ProviderEvent, ProviderFailureClassification,
    ProviderInstallProvenance, ProvisionedProviderInstallation, ProvisionedProviderLock,
    RunVersionLock,
};
pub use provider_prompt_provision::{
    ProviderPromptProvisionBinding, ProviderPromptProvisionCommandBinding,
    ProviderPromptProvisionPackageBinding,
};
pub use provider_prompt_readiness::{
    ProviderPromptReadinessBinding, ProviderPromptReadinessFailureBinding,
};
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
pub use sandbox_policy::{SandboxLaunchPolicy, SandboxWorkspaceAccess};
pub use tool_activity::{ToolActivity, ToolCategory, ToolPhase};
pub use tool_sources::{ToolSourceKind, ToolSourceRecord};
pub use turn_follow::TurnTerminal;
pub use workspace_git::{WorkspaceGitFileStatus, WorkspaceGitReport, WorkspaceGitWorktree};
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
