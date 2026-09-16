//! Stable value types shared by every public Gent crate.
mod agent_chat;
mod agent_chat_checkpoint;
mod agent_chat_command;
mod agent_chat_compaction;
mod agent_chat_conversation_config;
mod agent_chat_fork;
mod agent_chat_intent;
mod agent_chat_ledger;
mod agent_chat_prompt;
mod agent_chat_prompt_origin;
mod agent_chat_run_context;
mod agent_chat_sessions;
mod agent_chat_side_question;
mod agent_chat_switch;
mod agent_chat_terminal_settlement;
mod attachments;
mod automations;
mod bounded_text;
mod command_fingerprint;
#[cfg(test)]
mod command_fingerprint_tests;
#[cfg(test)]
mod contract_edge_tests;
mod conversation_activity;
mod conversation_artifact;
mod conversation_content;
mod conversation_context;
mod conversation_context_compaction;
mod conversation_links;
mod conversation_prompts;
mod conversations;
mod decision;
mod doctor;
mod event_page;
mod git_operations;
mod goal;
mod host_protocol;
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
mod token_usage;
mod tool_activity;
mod tool_sources;
mod turn_follow;
mod workspace_git;
mod workspaces;
pub use agent_chat::{
    AgentChatConversationDetail, AgentChatConversationSummary, AgentChatEffort, AgentChatMode,
    AgentChatProjectionEvent, AgentChatProjectionPage, AgentChatProjectionTail, AgentChatProvider,
    AgentChatRejection, AgentChatRun, AgentChatRunState, AgentChatSelection,
    AgentChatSelectionError, NormalizedTranscriptAppend, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, NormalizedTranscriptPage,
};
pub use agent_chat_checkpoint::{
    AgentChatCheckpointCapture, AgentChatCheckpointRestore, AgentChatCheckpointRestored,
    AgentChatFileCheckpoint, AgentChatFileCheckpointFile, AgentChatFileSnapshot,
};
pub use agent_chat_command::{
    AgentChatClientAction, AgentChatCommandAvailability, AgentChatCommandDescriptor,
    AgentChatCommandDispatch, AgentChatCommandIntent, AgentChatCommandOrigin,
    AgentChatCommandRejection, MAX_COMMAND_ARGUMENTS_BYTES, MAX_COMMAND_NAME_BYTES,
    MAX_COMMAND_TEXT_BYTES, slash_command, valid_command_name,
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
    AgentChatPromptError, AgentChatPromptSaved, PromptHoldReason, validate_tool_source_ids,
};
pub use agent_chat_prompt_origin::AgentChatPromptOrigin;
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
    AttachmentMetadata, AttachmentOperation, AttachmentReference, AttachmentState,
    AttachmentTransfer, TurnAttachment,
};
pub use automations::{
    AutomationAction, AutomationDefinition, AutomationId, AutomationNotifications, AutomationRun,
    AutomationRunId, AutomationRunStatus, AutomationRunSummary, AutomationTrigger,
};
pub use bounded_text::{
    INTERRUPTED_REPLY_EVENT_PREFIX, MAX_TRANSCRIPT_TEXT_BYTES, OVERSIZED_PROVIDER_FRAME_DIAGNOSTIC,
    OVERSIZED_PROVIDER_FRAME_NOTICE, PROVIDER_CONTEXT_COMPACTED_DIAGNOSTIC,
    PROVIDER_CONTEXT_COMPACTED_NOTICE, PROVIDER_CONTEXT_COMPACTION_FAILED_DIAGNOSTIC,
    PROVIDER_CONTEXT_COMPACTION_FAILED_NOTICE, PROVIDER_SESSION_RECOVERED_DIAGNOSTIC,
    PROVIDER_SESSION_RECOVERED_NOTICE, PROVIDER_SESSION_UNAVAILABLE_NOTICE, bounded_text,
};
pub use conversation_activity::{
    ActivityWorkKind, CONVERSATION_ACTIVITY_SCHEMA_VERSION, ConversationActivityFact,
    ConversationActivityPage, ConversationActivityScope, TurnTerminalCause,
};
pub use conversation_artifact::{
    ConversationArtifact, ConversationArtifactKind, ConversationArtifactStatus,
};
pub use conversation_content::{
    ConversationContentCursor, ConversationContentCursorError, ConversationContentEntry,
    ConversationContentPage,
};
pub use conversation_context::{ConversationContextSummary, FrozenConversationContext};
pub use conversation_context_compaction::{
    CONTEXT_COMPACTION_EVENT_KIND, CONTEXT_COMPACTION_FALLBACK_NOTICE,
    CONTEXT_COMPACTION_PARTIAL_NOTICE, ContextCompactionFact, ContextCompactionFailure,
    ContextCompactionPlan, ContextCompactionTrigger, ContextCoverageDigest, ContextSourceItem,
    ContextSourceRole, MAX_CONTEXT_SUMMARY_BYTES,
};
pub use conversation_links::{
    ConversationLink, ConversationMessageDelivery, ConversationThreadState, ConversationWaitResult,
    ConversationWaitTarget, LinkedConversationSummary, LinkedConversations,
    MAX_CONVERSATION_LABEL_BYTES, MAX_CONVERSATION_MESSAGE_PREVIEW_BYTES,
    MAX_CONVERSATION_WAIT_REPLY_BYTES, MAX_CONVERSATION_WAIT_SECONDS,
    MAX_CONVERSATION_WAIT_TARGETS, bounded_excerpt, valid_conversation_label,
};
pub use conversation_prompts::{ConversationMessage, ConversationPrompt};
pub use conversations::{
    ConversationArtifactSummary, ConversationListItem, ConversationRecord, ConversationRunStatus,
    ConversationStatus, ConversationTimeline, ConversationTimelineRun, DurableTurnPhase,
    TurnRecord,
};
pub use decision::{DecisionCommand, DecisionSettlement, DecisionSettlementPhase};
pub use doctor::{
    CompatibilityTrust, DependencyStatus, DoctorNextAction, DoctorReport, ExecutableIdentity,
    McpDoctorStatus, McpPermissionStatus, PrivateBridgeAvailability, PublicProviderStatus,
};
pub use event_page::EventPage;
pub use git_operations::{GitOperationKind, GitOperationPhase, GitOperationRecord};
pub use goal::{
    GOAL_SCHEMA_VERSION, GoalBinding, GoalContractError, GoalDispatchState, GoalProjection,
    GoalRecord, GoalReportOutcome, GoalStatus, GoalStatusReason, GoalTurnObservation,
    MAX_GOAL_NOTE_BYTES, MAX_GOAL_OBJECTIVE_BYTES, valid_goal_id, valid_goal_text,
};
pub use host_protocol::{
    CapabilitySet, Command, Event, HostEpoch, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN, Receipt,
    ReceiptId, ReceiptStatus,
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
    PermissionCategory, PermissionDenialReason, PermissionMode, PermissionRequest, PolicyRecord,
    PolicyScope, SandboxEnforcement,
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
    PromptAdmissionExit, ProviderPromptReadinessBinding, ProviderPromptReadinessFailureBinding,
};
pub use reviewed_plan::{
    ContextPolicy, MAX_PLAN_CONTENT_BYTES, PlanArtifact, PlanImplementation, PlanRevision,
    PlanStatus, PlanTurn, ReviewedPlanContractError, ReviewedPlanId, StartImplementationRequest,
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
pub use token_usage::TokenUsage;
pub use tool_activity::{ToolActivity, ToolCategory, ToolPhase};
pub use tool_sources::{ToolSourceKind, ToolSourceRecord};
pub use turn_follow::TurnTerminal;
pub use workspace_git::{
    WorkspaceGitBranch, WorkspaceGitCommit, WorkspaceGitFileStatus, WorkspaceGitRemoteStatus,
    WorkspaceGitReport, WorkspaceGitStashEntry, WorkspaceGitWorktree,
};
pub use workspaces::{RepositoryRecord, WorkspaceRecord, WorktreeRecord};
