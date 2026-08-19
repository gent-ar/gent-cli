// `RuntimeApi` implementation kept separate from daemon construction.

use gent_protocol::{
    AgentChatConversationFrame, AgentChatIntentFrame, AgentChatTranscriptFrame, AttachmentFrame,
    DecisionRecoveryEvidence, DecisionSubmission, DependencyActionRequest, DependencyActionResult,
    DependencyPlan, DependencyPlanRequest, GoalFrame, OrchestrationFrame, PermissionPolicyFrame,
    ReviewedPlanFrame,
};
use gent_types::{
    Command, ConversationContentCursor, ConversationContentPage, ConversationStatus,
    ConversationTimeline, DecisionCommand, DecisionSettlement, DoctorReport, EventPage,
    HostStatus, Receipt,
};

use crate::{agent_chat_api, api, attachment_api, decision_mapping};

impl api::RuntimeApi for RuntimeFacade {
    fn capabilities(&self) -> Result<CapabilitySet, String> {
        self.coordinator
            .status()
            .map(|status| status.capabilities)
            .map_err(|error| error.to_string())
    }

    fn status(&self) -> Result<HostStatus, String> {
        self.coordinator.status().map_err(|error| error.to_string())
    }

    fn submit(&self, command: Command) -> Result<Receipt, String> {
        self.coordinator
            .submit(&command)
            .map_err(|error| error.to_string())
    }

    fn read_event_page(&self, after_cursor: u64, limit: usize) -> Result<EventPage, String> {
        self.coordinator
            .read_event_page(after_cursor, limit)
            .map_err(|error| error.to_string())
    }

    fn agent_chat_intent(
        &self,
        frame: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, String> {
        let host_epoch = self
            .coordinator
            .status()
            .map_err(|error| error.to_string())?
            .host_epoch;
        if let Some(ingress) = &self.ordinary_prompt_ingress {
            let _permit = prompt_intent(&frame)
                .then(|| ingress.acquire_prompt())
                .transpose()
                .map_err(prompt_admission_error)?;
            let mut ingress = ingress.clone();
            agent_chat_api::exchange_with_wake(
                &self.agent_chat_conversations,
                &self.agent_chat_prompts,
                &self.agent_chat_switches,
                host_epoch,
                frame,
                &mut ingress,
            )
        } else {
            agent_chat_api::exchange(
                &self.agent_chat_conversations,
                &self.agent_chat_prompts,
                &self.agent_chat_switches,
                host_epoch,
                frame,
            )
        }
    }

    fn agent_chat_conversation(
        &self,
        frame: AgentChatConversationFrame,
    ) -> Result<AgentChatConversationFrame, String> {
        let reads = self
            .agent_chat_reads
            .as_ref()
            .ok_or_else(|| "agent-chat conversation reads are observer-disabled".to_owned())?;
        match frame {
            AgentChatConversationFrame::SummaryRequest { conversation_id } => reads
                .summary(&conversation_id)
                .map(AgentChatConversationFrame::Summary)
                .map_err(|error| error.to_string()),
            AgentChatConversationFrame::DetailRequest { conversation_id } => reads
                .detail(&conversation_id)
                .map(AgentChatConversationFrame::Detail)
                .map_err(|error| error.to_string()),
            AgentChatConversationFrame::Summary(_) | AgentChatConversationFrame::Detail(_) => {
                Err("agent-chat conversation response frames are server-only".into())
            }
        }
    }

    fn agent_chat_transcript(
        &self,
        frame: AgentChatTranscriptFrame,
    ) -> Result<AgentChatTranscriptFrame, String> {
        let reads = self
            .agent_chat_reads
            .as_ref()
            .ok_or_else(|| "agent-chat transcript reads are observer-disabled".to_owned())?;
        match frame {
            AgentChatTranscriptFrame::PageRequest {
                conversation_id,
                after_cursor,
                limit,
            } => reads
                .transcript(&conversation_id, after_cursor, limit)
                .map(AgentChatTranscriptFrame::Page)
                .map_err(|error| error.to_string()),
            AgentChatTranscriptFrame::Page(_) => {
                Err("agent-chat transcript response frames are server-only".into())
            }
        }
    }

    fn agent_chat_turn_follow(
        &self,
        request: gent_runtime::TurnFollowRequest,
    ) -> Result<gent_runtime::TurnFollowRead, String> {
        let source = self.turn_follow_source.as_ref().ok_or_else(|| {
            "agent-chat turn follow is unavailable while gentd is observer-disabled".to_owned()
        })?;
        TurnFollowService::read(source, &request).map_err(|error| error.to_string())
    }

    fn permission_policy(
        &self,
        frame: PermissionPolicyFrame,
    ) -> Result<PermissionPolicyFrame, String> {
        crate::permission_policy_api::exchange(&self.coordinator, frame)
    }

    fn goal(&self, frame: GoalFrame) -> Result<GoalFrame, String> {
        let host_epoch = self
            .coordinator
            .status()
            .map_err(|error| error.to_string())?
            .host_epoch;
        crate::goal_api::exchange(&self.goals, host_epoch, frame)
    }

    fn orchestration(&self, frame: OrchestrationFrame) -> Result<OrchestrationFrame, String> {
        crate::orchestration_api::exchange(&self.orchestration, frame)
    }

    fn reviewed_plan(&self, frame: ReviewedPlanFrame) -> Result<ReviewedPlanFrame, String> {
        crate::reviewed_plan_api::exchange(&self.reviewed_plans, frame)
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
            .map_err(|error| error.to_string())
    }

    fn attachment(&self, frame: AttachmentFrame) -> Result<AttachmentFrame, String> {
        attachment_api::handle(&self.attachments, frame)
    }

    fn runtime_update_check(
        &self,
        request: gent_types::RuntimeUpdateCheckRequest,
    ) -> Result<gent_types::RuntimeUpdateCheckReport, String> {
        self.runtime_update_checks
            .as_ref()
            .map(|checks| checks.check(request, crate::startup::unix_seconds()))
            .ok_or_else(|| "runtime update checks are observer-disabled".into())
    }

    fn runtime_maintenance(
        &self,
        request: gent_types::RuntimeMaintenanceRequest,
    ) -> Result<gent_types::RuntimeMaintenanceReport, String> {
        self.runtime_maintenance
            .read(&request)
            .map_err(|error| error.to_string())
    }

    fn submit_decision(&self, command: DecisionCommand) -> Result<DecisionSubmission, String> {
        self.coordinator
            .submit_decision(command)
            .map(decision_mapping::submission)
            .map_err(|error| error.to_string())
    }

    fn apply_decision_recovery(
        &self,
        decision_id: String,
        evidence: DecisionRecoveryEvidence,
    ) -> Result<DecisionSettlement, String> {
        self.coordinator
            .apply_decision_evidence(&decision_id, decision_mapping::recovery(evidence))
            .map_err(|error| error.to_string())
    }

    fn start_public_run(
        &self,
        request: gent_protocol::PublicRunStartRequest,
    ) -> Result<gent_protocol::PublicRunResponse, String> {
        self.public_runs
            .start(request)
            .map_err(|error| error.to_string())
    }

    fn resume_public_run(
        &self,
        request: gent_protocol::PublicRunResumeRequest,
    ) -> Result<gent_protocol::PublicRunResponse, String> {
        self.public_runs
            .resume(request)
            .map_err(|error| error.to_string())
    }

    fn interrupt_public_run(
        &self,
        request: gent_protocol::PublicRunInterruptRequest,
    ) -> Result<gent_protocol::PublicRunResponse, String> {
        self.public_runs
            .interrupt(request)
            .map_err(|error| error.to_string())
    }

    fn conversation_status(&self, conversation_id: &str) -> Result<ConversationStatus, String> {
        self.coordinator
            .conversation_status(conversation_id)
            .map_err(|error| error.to_string())
    }

    fn conversations(&self) -> Result<Vec<gent_types::ConversationListItem>, String> {
        self.coordinator
            .conversations()
            .map_err(|error| error.to_string())
    }

    fn conversation_timeline(&self, conversation_id: &str) -> Result<ConversationTimeline, String> {
        self.coordinator
            .conversation_timeline(conversation_id)
            .map_err(|error| error.to_string())
    }

    fn conversation_content(
        &self,
        conversation_id: &str,
        before: Option<ConversationContentCursor>,
        limit: u16,
    ) -> Result<ConversationContentPage, String> {
        self.coordinator
            .conversation_content(conversation_id, before.as_ref(), limit)
            .map_err(|error| error.to_string())
    }
}

fn prompt_intent(frame: &AgentChatIntentFrame) -> bool {
    matches!(
        frame,
        AgentChatIntentFrame::SendPrompt { .. } | AgentChatIntentFrame::QueuePrompt { .. }
    )
}

fn prompt_admission_error(
    error: crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError,
) -> String {
    match error {
        crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError::RecoveryInProgress => {
            "ordinary lifecycle recovery is still in progress".to_owned()
        }
        crate::ordinary_lifecycle_control::OrdinaryPromptAdmissionError::ShuttingDown => {
            "ordinary lifecycle is shutting down".to_owned()
        }
    }
}
