mod agent_chat;
mod agent_chat_checkpoint;
mod agent_chat_command;
mod agent_chat_compaction;
mod agent_chat_conversation_config;
mod agent_chat_failure_code;
mod agent_chat_fork;
mod agent_chat_intent;
mod agent_chat_ledger;
mod agent_chat_prompt;
mod agent_chat_prompt_origin;
mod agent_chat_rejection;
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
mod command_grant;
mod command_pattern;
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
pub mod internal_tools;
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
mod projection_window;
mod prompt_templates;
mod provider_auth;
mod provider_auth_handoff;
mod provider_install_progress;
mod provider_lifecycle_values;
mod provider_prompt_provision;
mod provider_prompt_readiness;
mod receipt_lookup;
mod reviewed_plan;
mod run_checkpoints;
mod run_lifecycle_fact;
mod runtime_maintenance;
mod runtime_update;
mod sandbox_gateway;
mod sandbox_launch;
#[cfg(test)]
mod sandbox_launch_tests;
mod sandbox_policy;
mod sandbox_provider_contract;
mod sandbox_resources;
mod token_usage;
mod tool_activity;
mod tool_sources;
mod turn_follow;
mod workspace_git;
mod workspaces;
pub use agent_chat::{
    AgentChatConversationDetail, AgentChatConversationSummary, AgentChatEffort, AgentChatMode,
    AgentChatProjectionEvent, AgentChatProjectionPage, AgentChatProjectionTail, AgentChatProvider,
    AgentChatQueueDelivery, AgentChatRun, AgentChatRunState, AgentChatSelection,
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
    AgentChatCommandRejection, AgentChatCommandRejectionDetails, MAX_COMMAND_ARGUMENTS_BYTES,
    MAX_COMMAND_NAME_BYTES, MAX_COMMAND_TEXT_BYTES, slash_command, valid_command_name,
};
pub use agent_chat_compaction::{AgentChatCompactionFact, AgentChatCompactionFailure};
pub use agent_chat_conversation_config::{
    AgentChatConversationConfigRecord, AgentChatConversationConfigUnsupportedField,
};
pub use agent_chat_failure_code::AgentChatFailureCode;
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
pub use agent_chat_rejection::AgentChatRejection;
pub use agent_chat_run_context::{AgentChatRunContext, AgentChatRunContextOrigin};
pub use agent_chat_sessions::{AgentChatSession, AgentChatSessionId};
pub use agent_chat_side_question::{
    AgentChatSideQuestion, AgentChatSideQuestionAsked, AgentChatSideQuestionCancel,
    AgentChatSideQuestionCancelled, AgentChatSideQuestionFailureCode, AgentChatSideQuestionOutcome,
    AgentChatSideQuestionRecord, AgentChatSideQuestionStatus,
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
pub use command_grant::{CommandGrant, CommandPatternError};
pub use command_pattern::{
    answered_command_grant, command_grant_for, command_grant_permits, parse_command_pattern,
    validate_command_grant,
};
pub use conversation_activity::{
    ActivityWorkKind, CONVERSATION_ACTIVITY_SCHEMA_VERSION, ContextClearCommand,
    ConversationActivityFact, ConversationActivityPage, ConversationActivityScope,
    ModelDownloadPhase, TurnTerminalCause, WorkspaceFileChange,
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
    Containment, PermissionCategory, PermissionDenialReason, PermissionMode, PermissionRequest,
    PolicyRecord, PolicyScope, SandboxEnforcement,
};
pub use projection_window::{
    ConversationTurnCursor, ConversationTurnCursorPage, MAX_PROJECTION_WINDOW_CARRY,
    MAX_TURN_CURSOR_PAGE, ProjectionWindowCarry,
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
pub use provider_auth_handoff::{
    ProviderAuthHandoff, ProviderAuthHandoffMethod, ProviderAuthHandoffState,
};
pub use provider_install_progress::ProviderInstallProgress;
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
pub use receipt_lookup::{ReceiptIntentKind, ReceiptLookup, ReceiptPromptOutcome};
pub use reviewed_plan::{
    ContextPolicy, ImplementationStarted, MAX_IMPLEMENTATION_INSTRUCTIONS_BYTES,
    MAX_PLAN_CONTENT_BYTES, PlanArtifact, PlanRevision, PlanStatus, PlanTurn,
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
pub use sandbox_gateway::{SandboxGatewayApi, SandboxProviderGateway};
pub use sandbox_provider_contract::{ContainedProviderContract, ContainedProviderRoute};
mod sandbox_permission_binding;
#[cfg(test)]
mod sandbox_shell_runtime_tests;
pub use sandbox_launch::{
    SandboxBackendId, SandboxCommandEgress, SandboxCpuSemantics, SandboxLaunchAttestation,
    SandboxLaunchContractError, SandboxLaunchProfile, SandboxNetworkPolicy, SandboxPrivateState,
    SandboxResourceLimits, SandboxShellRuntime, SandboxedLaunchRequest,
};
pub use sandbox_permission_binding::SandboxPermissionBinding;
pub use sandbox_policy::{SandboxLaunchPolicy, SandboxWorkspaceAccess};
pub use sandbox_resources::SandboxResourceGrant;
pub use token_usage::TokenUsage;
pub use tool_activity::{
    ToolActivity, ToolCategory, ToolPhase, operation_preview, operation_preview_with_limit,
};
pub use tool_sources::{ToolSourceKind, ToolSourceRecord};
pub use turn_follow::TurnTerminal;
pub use workspace_git::{
    WorkspaceGitBranch, WorkspaceGitCommit, WorkspaceGitFileStatus, WorkspaceGitRemoteStatus,
    WorkspaceGitReport, WorkspaceGitStashEntry, WorkspaceGitWorktree,
};
pub use workspaces::{RepositoryRecord, WorkspaceRecord, WorktreeRecord};

mod hidden_command;
pub use hidden_command::{console_command, hidden_command, hidden_command_with_flags};

mod windows_pipe_client;
pub use windows_pipe_client::windows_pipe_client_candidates;
