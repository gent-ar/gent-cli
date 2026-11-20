use crate::runtime_facade::RuntimeOwnerControlPort;
use gent_protocol::{
    AgentChatCheckpointFrame, AgentChatConversationConfigFrame, AgentChatConversationFrame,
    AgentChatIntentFrame, AgentChatProjectionDelta, AgentChatProjectionFrame,
    AgentChatSideQuestionFrame, AgentChatTranscriptFrame, AttachmentFrame, AutomationFrame,
    DecisionRecoveryEvidence, DecisionSubmission, DependencyActionRequest, DependencyActionResult,
    DependencyPlan, DependencyPlanRequest, ForgeConnectorFrame, GoalFrame, LocalModelFrame,
    OrchestrationFrame, PermissionPolicyFrame, ProjectionCursor, PromptProviderProvisionFrame,
    PromptTemplateFrame, ProviderAuthFrame, ProviderReadinessFrame, PublicRunInterruptRequest,
    PublicRunResponse, PublicRunResumeRequest, PublicRunStartRequest, ReviewedPlanFrame,
    WorkspaceDocumentsFrame, WorkspaceGitFrame,
};
use gent_runtime::{ConversationActivityRead, TurnFollowRead, TurnFollowRequest};
use gent_types::{
    CapabilitySet, Command, ConversationContentCursor, ConversationContentPage,
    ConversationListItem, ConversationStatus, ConversationTimeline, DecisionCommand,
    DecisionSettlement, DoctorReport, EventPage, HostStatus, OnboardingState, Receipt,
    RuntimeMaintenanceReport, RuntimeMaintenanceRequest, RuntimeUpdateCheckReport,
    RuntimeUpdateCheckRequest,
};

pub(crate) fn stamped_status(
    status: Result<HostStatus, gent_runtime::RuntimeError>,
) -> Result<HostStatus, String> {
    status
        .map(|status| HostStatus {
            executable_digest_sha256: executable_digest().map(str::to_owned),
            ..status
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn executable_digest() -> Option<&'static str> {
    static DIGEST: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    DIGEST
        .get_or_init(|| {
            crate::local_model_integrity::file_sha256(&std::env::current_exe().ok()?).ok()
        })
        .as_deref()
}

pub(crate) trait RuntimeApi: Clone + Send + Sync + 'static {
    fn runtime_owner_control_port(&self) -> Option<RuntimeOwnerControlPort> {
        None
    }
    fn agent_chat_permission_port(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::agent_chat_permission_api::AgentChatPermissionPort>> {
        None
    }
    fn model_catalog_port(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::runtime_facade::ModelCatalogPort>> {
        None
    }
    fn capabilities(&self) -> Result<CapabilitySet, String>;
    fn status(&self) -> Result<HostStatus, String>;
    fn submit(&self, command: Command) -> Result<Receipt, String>;
    fn read_event_page(&self, after_cursor: u64, limit: usize) -> Result<EventPage, String>;
    fn doctor(&self) -> DoctorReport;
    fn onboarding(&self) -> OnboardingState {
        OnboardingState::from_doctor(&self.doctor())
    }
    fn dependency_plan(&self, request: DependencyPlanRequest) -> DependencyPlan;
    fn dependency_action(
        &self,
        request: DependencyActionRequest,
    ) -> Result<DependencyActionResult, String>;
    fn attachment(&self, _: AttachmentFrame) -> Result<AttachmentFrame, String> {
        Err("attachments are unavailable for this runtime".into())
    }
    fn local_models(&self, _: LocalModelFrame) -> Result<LocalModelFrame, String> {
        Err("local models are unavailable for this runtime".into())
    }
    fn automations(&self, _: AutomationFrame) -> Result<AutomationFrame, String> {
        Err("automations are unavailable while gentd is observer-disabled".into())
    }
    fn agent_chat_sessions(
        &self,
        _: gent_protocol::AgentChatSessionFrame,
    ) -> Result<gent_protocol::AgentChatSessionFrame, String> {
        Err("agent chat sessions are unavailable while gentd is observer-disabled".into())
    }
    fn prompt_templates(&self, _: PromptTemplateFrame) -> Result<PromptTemplateFrame, String> {
        Err("prompt templates are unavailable for this runtime".into())
    }
    fn workspace_documents(
        &self,
        _: WorkspaceDocumentsFrame,
    ) -> Result<WorkspaceDocumentsFrame, String> {
        Err("workspace documents are unavailable for this runtime".into())
    }
    fn workspace_git(
        &self,
        _: WorkspaceGitFrame,
    ) -> Result<WorkspaceGitFrame, crate::agent_chat_intent_error::AgentChatIntentError> {
        Err(crate::workspace_git_api::rejected(
            "workspace git is unavailable for this runtime",
        ))
    }
    fn forge_connectors(&self, _: ForgeConnectorFrame) -> Result<ForgeConnectorFrame, String> {
        Err("Forge connectors are unavailable while gentd is observer-disabled".into())
    }
    fn start_local_model_download(
        &self,
        _: &str,
    ) -> Result<crate::local_model_jobs::DownloadStart, gent_protocol::LocalModelDownloadFailure>
    {
        Err(gent_protocol::LocalModelDownloadFailure::StorageUnavailable)
    }
    fn runtime_update_check(
        &self,
        _: RuntimeUpdateCheckRequest,
    ) -> Result<RuntimeUpdateCheckReport, String> {
        Err("runtime update checks are observer-disabled".into())
    }
    fn runtime_maintenance(
        &self,
        _: RuntimeMaintenanceRequest,
    ) -> Result<RuntimeMaintenanceReport, String> {
        Err("runtime maintenance is observer-disabled".into())
    }
    fn submit_decision(&self, command: DecisionCommand) -> Result<DecisionSubmission, String>;
    fn apply_decision_recovery(
        &self,
        decision_id: String,
        evidence: DecisionRecoveryEvidence,
    ) -> Result<DecisionSettlement, String>;
    fn start_public_run(&self, request: PublicRunStartRequest)
    -> Result<PublicRunResponse, String>;
    fn resume_public_run(
        &self,
        request: PublicRunResumeRequest,
    ) -> Result<PublicRunResponse, String>;
    fn interrupt_public_run(
        &self,
        request: PublicRunInterruptRequest,
    ) -> Result<PublicRunResponse, String>;
    fn agent_chat_intent(
        &self,
        _: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, crate::agent_chat_intent_error::AgentChatIntentError>
    {
        Err("agent chat is unavailable while gentd is observer-disabled".into())
    }
    fn agent_chat_command(
        &self,
        _: gent_protocol::agent_chat_commands::AgentChatCommandFrame,
    ) -> Result<
        gent_protocol::agent_chat_commands::AgentChatCommandFrame,
        crate::agent_chat_intent_error::AgentChatIntentError,
    > {
        Err("agent-chat commands are unavailable while gentd is observer-disabled".into())
    }
    fn agent_chat_conversation(
        &self,
        _: AgentChatConversationFrame,
    ) -> Result<AgentChatConversationFrame, crate::agent_chat_intent_error::AgentChatIntentError>
    {
        Err(crate::agent_chat_intent_error::AgentChatIntentError::coded(
            "agentChatReadUnavailable",
            "agent-chat conversation reads are unavailable while gentd is observer-disabled",
        ))
    }
    fn agent_chat_transcript(
        &self,
        _: AgentChatTranscriptFrame,
    ) -> Result<AgentChatTranscriptFrame, crate::agent_chat_intent_error::AgentChatIntentError>
    {
        Err(crate::agent_chat_intent_error::AgentChatIntentError::coded(
            "agentChatReadUnavailable",
            "agent-chat transcript reads are unavailable while gentd is observer-disabled",
        ))
    }
    fn agent_chat_projection(
        &self,
        _: AgentChatProjectionFrame,
    ) -> Result<AgentChatProjectionFrame, crate::agent_chat_intent_error::AgentChatIntentError>
    {
        Err(crate::agent_chat_intent_error::AgentChatIntentError::coded(
            "agentChatProjectionUnavailable",
            "agent-chat projection is unavailable while gentd is observer-disabled",
        ))
    }
    fn agent_chat_projection_follow(
        &self,
        _: &str,
        _: &ProjectionCursor,
    ) -> Result<Vec<AgentChatProjectionDelta>, String> {
        Err("agent-chat projection is unavailable while gentd is observer-disabled".into())
    }
    fn provider_readiness(
        &self,
        _: ProviderReadinessFrame,
    ) -> Result<ProviderReadinessFrame, String> {
        Err("provider readiness is unavailable while gentd is observer-disabled".into())
    }
    fn prompt_provider_provision(
        &self,
        _: PromptProviderProvisionFrame,
        _: &dyn Fn(gent_types::ProviderInstallProgress),
    ) -> Result<PromptProviderProvisionFrame, String> {
        Err("prompt provider provisioning is unavailable while gentd is observer-disabled".into())
    }
    fn ledger_commits(&self) -> tokio::sync::watch::Receiver<u64> {
        tokio::sync::watch::channel(0).1
    }
    fn agent_chat_turn_follow(&self, _: TurnFollowRequest) -> Result<TurnFollowRead, String> {
        Err("agent-chat turn follow is unavailable while gentd is observer-disabled".into())
    }
    fn permission_policy(
        &self,
        _: PermissionPolicyFrame,
    ) -> Result<PermissionPolicyFrame, crate::agent_chat_intent_error::AgentChatIntentError> {
        Err(crate::permission_policy_api::rejected(
            "permission policy is unavailable for this runtime",
        ))
    }
    fn agent_chat_conversation_config(
        &self,
        _: AgentChatConversationConfigFrame,
    ) -> Result<AgentChatConversationConfigFrame, String> {
        Err("agent-chat conversation config is unavailable for this runtime".into())
    }
    fn agent_chat_checkpoint(
        &self,
        _: AgentChatCheckpointFrame,
    ) -> Result<AgentChatCheckpointFrame, String> {
        Err("agent-chat checkpoint is unavailable for this runtime".into())
    }
    fn agent_chat_side_question(
        &self,
        _: AgentChatSideQuestionFrame,
    ) -> Result<AgentChatSideQuestionFrame, crate::agent_chat_intent_error::AgentChatIntentError>
    {
        Err(crate::agent_chat_intent_error::AgentChatIntentError::coded(
            "agentChatSideQuestionRejected",
            "agent-chat side question is unavailable for this runtime",
        ))
    }
    fn provider_auth(&self, _: ProviderAuthFrame) -> Result<ProviderAuthFrame, String> {
        Err("provider authentication is unavailable while gentd is observer-disabled".into())
    }
    fn reviewed_plan(
        &self,
        _: ReviewedPlanFrame,
    ) -> Result<ReviewedPlanFrame, crate::agent_chat_intent_error::AgentChatIntentError> {
        Err(crate::reviewed_plan_api::rejected(
            "reviewed plans are unavailable while gentd is observer-disabled",
        ))
    }
    fn orchestration(&self, _: OrchestrationFrame) -> Result<OrchestrationFrame, String> {
        Err("orchestration is unavailable while gentd is observer-disabled".into())
    }
    fn goal(&self, _: GoalFrame) -> Result<GoalFrame, String> {
        Err("goals are unavailable while gentd is observer-disabled".into())
    }
    fn conversation_status(&self, conversation_id: &str) -> Result<ConversationStatus, String>;
    fn conversations(&self) -> Result<Vec<ConversationListItem>, String> {
        Err("conversation discovery is unavailable for this runtime".into())
    }
    fn conversation_timeline(&self, conversation_id: &str) -> Result<ConversationTimeline, String>;
    fn conversation_activity(
        &self,
        _: &str,
        _: &str,
        _: u64,
    ) -> Result<ConversationActivityRead, String> {
        Err("conversation activity is unavailable for this runtime".into())
    }
    fn conversation_content(
        &self,
        _: &str,
        _: Option<ConversationContentCursor>,
        _: u16,
    ) -> Result<ConversationContentPage, String> {
        Err("conversation content is unavailable for this runtime".into())
    }
}
