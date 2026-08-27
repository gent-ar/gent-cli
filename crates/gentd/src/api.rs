//! Daemon-facing runtime port. Transport adapters depend only on this boundary.

use gent_protocol::{
    AgentChatCheckpointFrame, AgentChatConversationConfigFrame, AgentChatConversationFrame,
    AgentChatIntentFrame, AgentChatSideQuestionFrame, AgentChatTranscriptFrame, AttachmentFrame,
    AutomationFrame, DecisionRecoveryEvidence, DecisionSubmission, DependencyActionRequest,
    DependencyActionResult, DependencyPlan, DependencyPlanRequest, ForgeConnectorFrame, GoalFrame,
    LocalModelFrame, OrchestrationFrame, PermissionPolicyFrame, PromptProviderProvisionFrame,
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

pub(crate) trait RuntimeApi: Clone + Send + Sync + 'static {
    fn agent_chat_permission_port(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::agent_chat_permission_api::AgentChatPermissionPort>> {
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
    /// Handles a capability-gated attachment transfer frame.
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
    fn workspace_git(&self, _: WorkspaceGitFrame) -> Result<WorkspaceGitFrame, String> {
        Err("workspace git is unavailable for this runtime".into())
    }
    fn forge_connectors(&self, _: ForgeConnectorFrame) -> Result<ForgeConnectorFrame, String> {
        Err("Forge connectors are unavailable while gentd is observer-disabled".into())
    }
    fn begin_local_model_download(
        &self,
        _: &str,
    ) -> Result<crate::standalone_authority_composition::LocalModelDownloadStart, String> {
        Err("local-model downloads are unavailable for this runtime".into())
    }
    fn finish_local_model_download(&self, _: &str) {}
    fn publish_local_model_frame(&self, _: LocalModelFrame) -> Result<(), String> {
        Ok(())
    }
    /// Reads a separately configured, signed cached release report.
    fn runtime_update_check(
        &self,
        _: RuntimeUpdateCheckRequest,
    ) -> Result<RuntimeUpdateCheckReport, String> {
        Err("runtime update checks are observer-disabled".into())
    }
    /// Reads one separately enabled durable update-maintenance attempt.
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
    /// Handles a finite authority-gated agent-chat exchange.
    ///
    /// The transport validates that every response is correlated to `request`. Observer
    /// implementations keep this default and therefore cannot accept chat commands merely by
    /// negotiating the protocol capability.
    fn agent_chat_intent(
        &self,
        _: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, String> {
        Err("agent chat is unavailable while gentd is observer-disabled".into())
    }
    /// Reads a public, provider-neutral conversation view only in a composed chat authority.
    ///
    /// The hard observer deliberately keeps this unavailable and does not advertise its
    /// capability, even though the protocol DTOs are stable for future clients.
    fn agent_chat_conversation(
        &self,
        _: AgentChatConversationFrame,
    ) -> Result<AgentChatConversationFrame, String> {
        Err("agent-chat conversation reads are unavailable while gentd is observer-disabled".into())
    }
    /// Reads a normalized transcript page only in a composed chat authority.
    fn agent_chat_transcript(
        &self,
        _: AgentChatTranscriptFrame,
    ) -> Result<AgentChatTranscriptFrame, String> {
        Err("agent-chat transcript reads are unavailable while gentd is observer-disabled".into())
    }
    /// Assesses one exact current run only in an approved private readiness composition.
    fn provider_readiness(
        &self,
        _: ProviderReadinessFrame,
    ) -> Result<ProviderReadinessFrame, String> {
        Err("provider readiness is unavailable while gentd is observer-disabled".into())
    }
    /// Confirms one daemon-derived provider-install review only in a private authority profile.
    ///
    /// This default refuses before package policy, Node, npm, or provider code can be reached.
    fn prompt_provider_provision(
        &self,
        _: PromptProviderProvisionFrame,
    ) -> Result<PromptProviderProvisionFrame, String> {
        Err("prompt provider provisioning is unavailable while gentd is observer-disabled".into())
    }
    /// Reads one epoch-fenced page of one exact normalized turn in an approved composition.
    ///
    /// The default keeps observer and persistence-only profiles unable to follow a turn.
    fn agent_chat_turn_follow(&self, _: TurnFollowRequest) -> Result<TurnFollowRead, String> {
        Err("agent-chat turn follow is unavailable while gentd is observer-disabled".into())
    }
    /// Reads or appends one local, provider-neutral permission-policy revision.
    fn permission_policy(&self, _: PermissionPolicyFrame) -> Result<PermissionPolicyFrame, String> {
        Err("permission policy is unavailable for this runtime".into())
    }
    /// Reads or appends one durable per-conversation advanced-launch-configuration revision.
    fn agent_chat_conversation_config(
        &self,
        _: AgentChatConversationConfigFrame,
    ) -> Result<AgentChatConversationConfigFrame, String> {
        Err("agent-chat conversation config is unavailable for this runtime".into())
    }
    /// Captures, lists, or restores a durable per-turn file checkpoint.
    fn agent_chat_checkpoint(
        &self,
        _: AgentChatCheckpointFrame,
    ) -> Result<AgentChatCheckpointFrame, String> {
        Err("agent-chat checkpoint is unavailable for this runtime".into())
    }
    /// Asks, cancels, or lists bounded, provider-neutral side questions.
    fn agent_chat_side_question(
        &self,
        _: AgentChatSideQuestionFrame,
    ) -> Result<AgentChatSideQuestionFrame, String> {
        Err("agent-chat side question is unavailable for this runtime".into())
    }
    /// Handles a secret-free provider-auth frame only in a future authority composition.
    ///
    /// The observer default refuses before any provider discovery, browser launch, or login
    /// process can be reached.
    fn provider_auth(&self, _: ProviderAuthFrame) -> Result<ProviderAuthFrame, String> {
        Err("provider authentication is unavailable while gentd is observer-disabled".into())
    }
    /// Handles reviewed-plan reads and user decisions only in a future authority composition.
    fn reviewed_plan(&self, _: ReviewedPlanFrame) -> Result<ReviewedPlanFrame, String> {
        Err("reviewed plans are unavailable while gentd is observer-disabled".into())
    }
    /// Reads or mutates a Gent-owned task graph only in a future authority composition.
    ///
    /// The hard observer refuses before a graph read/write, worktree lease, or provider path.
    fn orchestration(&self, _: OrchestrationFrame) -> Result<OrchestrationFrame, String> {
        Err("orchestration is unavailable while gentd is observer-disabled".into())
    }
    /// Handles provider-neutral `/goal` records only in an approved chat persistence profile.
    ///
    /// This default deliberately refuses before a ledger write or any provider path can occur.
    fn goal(&self, _: GoalFrame) -> Result<GoalFrame, String> {
        Err("goals are unavailable while gentd is observer-disabled".into())
    }
    fn conversation_status(&self, conversation_id: &str) -> Result<ConversationStatus, String>;
    fn conversations(&self) -> Result<Vec<ConversationListItem>, String> {
        Err("conversation discovery is unavailable for this runtime".into())
    }
    fn conversation_timeline(&self, conversation_id: &str) -> Result<ConversationTimeline, String>;
    /// Returns activity only for a future authority-gated runtime composition.
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
