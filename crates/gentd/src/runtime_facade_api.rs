use std::sync::Arc;

use gent_protocol::{
    AgentChatCheckpointFrame, AgentChatConversationConfigFrame, AgentChatConversationFrame,
    AgentChatIntentFrame, AgentChatProjectionDelta, AgentChatProjectionFrame,
    AgentChatSessionFrame, AgentChatSideQuestionFrame, AgentChatTranscriptFrame, AttachmentFrame,
    AutomationFrame, DecisionRecoveryEvidence, DecisionSubmission, DependencyActionRequest,
    DependencyActionResult, DependencyPlan, DependencyPlanRequest, ForgeConnectorFrame, GoalFrame,
    LocalModelDownloadFailure, LocalModelFrame, OrchestrationFrame, PermissionPolicyFrame,
    ProjectionCursor, PromptProviderProvisionFrame, PromptTemplateFrame, ProviderAuthFrame,
    ProviderReadinessFrame, PublicRunInterruptRequest, PublicRunResponse, PublicRunResumeRequest,
    PublicRunStartRequest, ReviewedPlanFrame, WorkspaceDocumentsFrame, WorkspaceGitFrame,
};
use gent_runtime::{
    ConversationActivityRead, TurnFollowRead, TurnFollowRequest, TurnFollowService,
};
use gent_types::{
    CapabilitySet, Command, ConversationContentCursor, ConversationContentPage,
    ConversationListItem, ConversationStatus, ConversationTimeline, DecisionCommand,
    DecisionSettlement, DoctorReport, EventPage, HostStatus, Receipt, RuntimeMaintenanceReport,
    RuntimeMaintenanceRequest, RuntimeUpdateCheckReport, RuntimeUpdateCheckRequest,
};

use super::{ModelCatalogPort, RuntimeFacade, chat_reads};
use crate::{
    agent_chat_checkpoint_api, agent_chat_conversation_config_api,
    agent_chat_intent_error::AgentChatIntentError,
    agent_chat_permission_api::AgentChatPermissionPort,
    agent_chat_sessions_api,
    api::RuntimeApi,
    attachment_api, decision_mapping, forge_api, goal_api,
    local_model_jobs::DownloadStart,
    orchestration_api, permission_policy_api, reviewed_plan_api,
    workspace_git_api::{self, WorkspaceGitRejection},
};

impl RuntimeApi for RuntimeFacade {
    fn agent_chat_permission_port(&self) -> Option<Arc<dyn AgentChatPermissionPort>> {
        self.agent_chat_permission_port.clone()
    }
    fn model_catalog_port(&self) -> Option<Arc<dyn ModelCatalogPort>> {
        self.model_catalog.clone()
    }
    fn provider_auth(&self, frame: ProviderAuthFrame) -> Result<ProviderAuthFrame, String> {
        self.exchange_provider_auth(frame)
    }
    fn capabilities(&self) -> Result<CapabilitySet, String> {
        self.coordinator
            .status()
            .map(|status| status.capabilities)
            .map_err(display)
    }
    fn status(&self) -> Result<HostStatus, String> {
        crate::api::stamped_status(self.coordinator.status())
    }
    fn submit(&self, command: Command) -> Result<Receipt, String> {
        self.coordinator.submit(&command).map_err(display)
    }
    fn read_event_page(&self, after_cursor: u64, limit: usize) -> Result<EventPage, String> {
        self.coordinator
            .read_event_page(after_cursor, limit)
            .map_err(display)
    }
    fn agent_chat_intent(
        &self,
        frame: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        self.exchange_public_agent_chat_intent(frame)
    }
    fn agent_chat_command(
        &self,
        frame: gent_protocol::agent_chat_commands::AgentChatCommandFrame,
    ) -> Result<gent_protocol::agent_chat_commands::AgentChatCommandFrame, AgentChatIntentError>
    {
        self.exchange_agent_chat_command(frame)
    }
    fn agent_chat_conversation(
        &self,
        frame: AgentChatConversationFrame,
    ) -> Result<AgentChatConversationFrame, AgentChatIntentError> {
        chat_reads::conversation(self, frame)
    }
    fn agent_chat_transcript(
        &self,
        frame: AgentChatTranscriptFrame,
    ) -> Result<AgentChatTranscriptFrame, AgentChatIntentError> {
        chat_reads::transcript(self, frame)
    }
    fn agent_chat_projection(
        &self,
        frame: AgentChatProjectionFrame,
    ) -> Result<AgentChatProjectionFrame, AgentChatIntentError> {
        chat_reads::projection(self, frame)
    }
    fn agent_chat_projection_follow(
        &self,
        conversation_id: &str,
        after_cursor: &ProjectionCursor,
    ) -> Result<Vec<AgentChatProjectionDelta>, String> {
        chat_reads::follow(self, conversation_id, after_cursor)
    }
    fn automations(&self, frame: AutomationFrame) -> Result<AutomationFrame, String> {
        self.exchange_automation(frame)
    }
    fn agent_chat_sessions(
        &self,
        frame: AgentChatSessionFrame,
    ) -> Result<AgentChatSessionFrame, String> {
        agent_chat_sessions_api::exchange(&self.agent_chat_sessions, frame)
    }
    fn forge_connectors(&self, frame: ForgeConnectorFrame) -> Result<ForgeConnectorFrame, String> {
        forge_api::exchange(&self.coordinator, frame)
    }
    fn provider_readiness(
        &self,
        frame: ProviderReadinessFrame,
    ) -> Result<ProviderReadinessFrame, String> {
        self.assess_provider_readiness(frame)
    }
    fn prompt_provider_provision(
        &self,
        frame: PromptProviderProvisionFrame,
    ) -> Result<PromptProviderProvisionFrame, String> {
        self.confirm_prompt_provider_provision(frame)
    }
    fn agent_chat_turn_follow(&self, request: TurnFollowRequest) -> Result<TurnFollowRead, String> {
        let source = self.turn_follow_source.as_ref().ok_or_else(|| {
            "agent-chat turn follow is unavailable while gentd is observer-disabled".to_owned()
        })?;
        TurnFollowService::read(source, &request).map_err(display)
    }
    fn conversation_activity(
        &self,
        conversation_id: &str,
        run_id: &str,
        after_cursor: u64,
    ) -> Result<ConversationActivityRead, String> {
        self.conversation_activity
            .read(conversation_id, run_id, after_cursor)
            .map_err(display)
    }
    fn permission_policy(
        &self,
        frame: PermissionPolicyFrame,
    ) -> Result<PermissionPolicyFrame, String> {
        permission_policy_api::exchange(&self.coordinator, frame)
    }
    fn agent_chat_conversation_config(
        &self,
        frame: AgentChatConversationConfigFrame,
    ) -> Result<AgentChatConversationConfigFrame, String> {
        agent_chat_conversation_config_api::exchange(
            &self.coordinator,
            self.agent_chat_reads.as_ref(),
            frame,
        )
    }
    fn agent_chat_checkpoint(
        &self,
        frame: AgentChatCheckpointFrame,
    ) -> Result<AgentChatCheckpointFrame, String> {
        agent_chat_checkpoint_api::exchange(&self.agent_chat_checkpoints, self.host_epoch()?, frame)
    }
    fn agent_chat_side_question(
        &self,
        frame: AgentChatSideQuestionFrame,
    ) -> Result<AgentChatSideQuestionFrame, String> {
        self.exchange_side_question(frame)
    }
    fn goal(&self, frame: GoalFrame) -> Result<GoalFrame, String> {
        goal_api::exchange(&self.goals, self.host_epoch()?, frame)
    }
    fn prompt_templates(&self, frame: PromptTemplateFrame) -> Result<PromptTemplateFrame, String> {
        self.exchange_prompt_template(frame)
    }
    fn workspace_documents(
        &self,
        frame: WorkspaceDocumentsFrame,
    ) -> Result<WorkspaceDocumentsFrame, String> {
        self.list_workspace_documents(frame)
    }
    fn workspace_git(
        &self,
        frame: WorkspaceGitFrame,
    ) -> Result<WorkspaceGitFrame, WorkspaceGitRejection> {
        workspace_git_api::exchange(&self.coordinator, frame)
    }
    fn orchestration(&self, frame: OrchestrationFrame) -> Result<OrchestrationFrame, String> {
        orchestration_api::exchange(&self.orchestration, frame)
    }
    fn reviewed_plan(&self, frame: ReviewedPlanFrame) -> Result<ReviewedPlanFrame, String> {
        reviewed_plan_api::exchange(&self.reviewed_plans, frame)
    }
    fn doctor(&self) -> DoctorReport {
        self.dependencies.doctor()
    }
    fn dependency_plan(&self, request: DependencyPlanRequest) -> DependencyPlan {
        self.dependencies.plan(request)
    }
    fn dependency_action(
        &self,
        request: DependencyActionRequest,
    ) -> Result<DependencyActionResult, String> {
        let plan = self.dependencies.plan(DependencyPlanRequest {
            provider: request.provider,
            action: request.action,
        });
        self.dependency_actions
            .execute(&request, &plan)
            .map_err(display)
    }
    fn attachment(&self, frame: AttachmentFrame) -> Result<AttachmentFrame, String> {
        attachment_api::handle(&self.attachments, self.host_epoch()?, frame)
    }
    fn local_models(&self, frame: LocalModelFrame) -> Result<LocalModelFrame, String> {
        self.local_model_frame(frame)
    }
    fn start_local_model_download(
        &self,
        model_id: &str,
    ) -> Result<DownloadStart, LocalModelDownloadFailure> {
        self.start_background_model_download(model_id)
    }
    fn runtime_update_check(
        &self,
        request: RuntimeUpdateCheckRequest,
    ) -> Result<RuntimeUpdateCheckReport, String> {
        Ok(self.runtime_update_checks.as_ref().map_or_else(
            || crate::runtime_update_config::packaged::metadata_unavailable(request.channel),
            |checks| checks.check(request, crate::startup::unix_seconds()),
        ))
    }
    fn runtime_maintenance(
        &self,
        request: RuntimeMaintenanceRequest,
    ) -> Result<RuntimeMaintenanceReport, String> {
        self.runtime_maintenance.read(&request).map_err(display)
    }
    fn submit_decision(&self, command: DecisionCommand) -> Result<DecisionSubmission, String> {
        self.coordinator
            .submit_decision(command)
            .map(decision_mapping::submission)
            .map_err(display)
    }
    fn apply_decision_recovery(
        &self,
        decision_id: String,
        evidence: DecisionRecoveryEvidence,
    ) -> Result<DecisionSettlement, String> {
        self.coordinator
            .apply_decision_evidence(&decision_id, decision_mapping::recovery(evidence))
            .map_err(display)
    }
    fn start_public_run(
        &self,
        request: PublicRunStartRequest,
    ) -> Result<PublicRunResponse, String> {
        self.public_runs.start(request).map_err(display)
    }
    fn resume_public_run(
        &self,
        request: PublicRunResumeRequest,
    ) -> Result<PublicRunResponse, String> {
        self.public_runs.resume(request).map_err(display)
    }
    fn interrupt_public_run(
        &self,
        request: PublicRunInterruptRequest,
    ) -> Result<PublicRunResponse, String> {
        self.public_runs.interrupt(request).map_err(display)
    }
    fn conversation_status(&self, conversation_id: &str) -> Result<ConversationStatus, String> {
        self.coordinator
            .conversation_status(conversation_id)
            .map_err(display)
    }
    fn conversations(&self) -> Result<Vec<ConversationListItem>, String> {
        self.coordinator.conversations().map_err(display)
    }
    fn conversation_timeline(&self, conversation_id: &str) -> Result<ConversationTimeline, String> {
        self.coordinator
            .conversation_timeline(conversation_id)
            .map_err(display)
    }
    fn conversation_content(
        &self,
        conversation_id: &str,
        before: Option<ConversationContentCursor>,
        limit: u16,
    ) -> Result<ConversationContentPage, String> {
        self.coordinator
            .conversation_content(conversation_id, before.as_ref(), limit)
            .map_err(display)
    }
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}
