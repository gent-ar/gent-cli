use crate::{agent_chat_api, api, attachment_api, decision_mapping};
use gent_protocol::{
    AgentChatCheckpointFrame, AgentChatConversationConfigFrame, AgentChatConversationFrame,
    AgentChatIntentFrame, AgentChatSideQuestionFrame, AgentChatTranscriptFrame, AttachmentFrame,
    AutomationFrame, ForgeConnectorFrame,
    DecisionRecoveryEvidence, DecisionSubmission, DependencyActionRequest, DependencyActionResult,
    DependencyPlan, DependencyPlanRequest, GoalFrame, LocalModelFrame, OrchestrationFrame,
    PermissionPolicyFrame, ProviderReadinessFrame, ReviewedPlanFrame,
};
use gent_types::{
    Command, ConversationContentCursor, ConversationContentPage, ConversationStatus,
    ConversationTimeline, DecisionCommand, DecisionSettlement, DoctorReport, EventPage, HostStatus,
    Receipt,
};
impl api::RuntimeApi for RuntimeFacade {
    fn agent_chat_permission_port(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::agent_chat_permission_api::AgentChatPermissionPort>> {
        self.agent_chat_permission_port.clone()
    }
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
        if let Some(reply) = self.interrupt_intent(host_epoch, frame.clone())? {
            return Ok(vec![reply]);
        }
        if let Some(ingress) = &self.ordinary_prompt_ingress {
            let _permit = prompt_admission::prompt_intent(&frame)
                .then(|| ingress.acquire_prompt())
                .transpose()
                .map_err(prompt_admission::prompt_admission_error)?;
            let mut ingress = ingress.clone();
            agent_chat_api::exchange_with_wake(
                &self.agent_chat_conversations,
                &self.agent_chat_prompts,
                &self.agent_chat_switches,
                &self.agent_chat_forks,
                host_epoch,
                frame,
                &mut ingress,
            )
        } else {
            agent_chat_api::exchange(
                &self.agent_chat_conversations,
                &self.agent_chat_prompts,
                &self.agent_chat_switches,
                &self.agent_chat_forks,
                host_epoch,
                frame,
            )
        }
    }
    fn agent_chat_conversation(
        &self,
        frame: AgentChatConversationFrame,
    ) -> Result<AgentChatConversationFrame, String> {
        chat_reads::conversation(self, frame)
    }
    fn agent_chat_transcript(
        &self,
        frame: AgentChatTranscriptFrame,
    ) -> Result<AgentChatTranscriptFrame, String> {
        chat_reads::transcript(self, frame)
    }
    fn automations(&self, frame: AutomationFrame) -> Result<AutomationFrame, String> {
        match frame {
            AutomationFrame::RunRequest {
                request_id,
                automation_id,
            } => self.run_automation(request_id, automation_id),
            frame => crate::automation_api::exchange(&self.automations, frame),
        }
    }
    fn agent_chat_sessions(
        &self,
        frame: gent_protocol::AgentChatSessionFrame,
    ) -> Result<gent_protocol::AgentChatSessionFrame, String> {
        crate::agent_chat_sessions_api::exchange(&self.agent_chat_sessions, frame)
    }
    fn forge_connectors(&self, frame: ForgeConnectorFrame) -> Result<ForgeConnectorFrame, String> {
        crate::forge_api::exchange(&self.coordinator, frame)
    }
    fn provider_readiness(
        &self,
        frame: ProviderReadinessFrame,
    ) -> Result<ProviderReadinessFrame, String> {
        self.provider_readiness
            .as_ref()
            .ok_or_else(|| "provider readiness is observer-disabled".to_owned())?
            .assess(frame)
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
    fn conversation_activity(
        &self,
        conversation_id: &str,
        run_id: &str,
        after_cursor: u64,
    ) -> Result<gent_runtime::ConversationActivityRead, String> {
        self.conversation_activity
            .read(conversation_id, run_id, after_cursor)
            .map_err(|error| error.to_string())
    }
    fn permission_policy(
        &self,
        frame: PermissionPolicyFrame,
    ) -> Result<PermissionPolicyFrame, String> {
        crate::permission_policy_api::exchange(&self.coordinator, frame)
    }
    fn agent_chat_conversation_config(
        &self,
        frame: AgentChatConversationConfigFrame,
    ) -> Result<AgentChatConversationConfigFrame, String> {
        crate::agent_chat_conversation_config_api::exchange(
            &self.coordinator,
            self.agent_chat_reads.as_ref(),
            frame,
        )
    }
    fn agent_chat_checkpoint(
        &self,
        frame: AgentChatCheckpointFrame,
    ) -> Result<AgentChatCheckpointFrame, String> {
        let host_epoch = self
            .coordinator
            .status()
            .map_err(|error| error.to_string())?
            .host_epoch;
        crate::agent_chat_checkpoint_api::exchange(&self.agent_chat_checkpoints, host_epoch, frame)
    }
    fn agent_chat_side_question(
        &self,
        frame: AgentChatSideQuestionFrame,
    ) -> Result<AgentChatSideQuestionFrame, String> {
        let host_epoch = self
            .coordinator
            .status()
            .map_err(|error| error.to_string())?
            .host_epoch;
        let is_ask = matches!(frame, AgentChatSideQuestionFrame::AskSideQuestion { .. });
        let response = crate::agent_chat_side_question_api::exchange(
            &self.agent_chat_side_questions,
            host_epoch,
            frame,
        )?;
        if is_ask {
            if let AgentChatSideQuestionFrame::Asked { ref record, .. } = response {
                if record.status == gent_types::AgentChatSideQuestionStatus::Pending {
                    crate::agent_chat_side_question_worker::dispatch(
                        self.agent_chat_side_questions.clone(),
                        self.agent_chat_side_question_runners.clone(),
                        self.local_model_events.clone(),
                        host_epoch,
                        record.side_question_id.clone(),
                        record.conversation_id.0.clone(),
                        record.question.clone(),
                    );
                }
            }
        }
        Ok(response)
    }
    fn goal(&self, frame: GoalFrame) -> Result<GoalFrame, String> {
        let host_epoch = self
            .coordinator
            .status()
            .map_err(|error| error.to_string())?
            .host_epoch;
        crate::goal_api::exchange(&self.goals, host_epoch, frame)
    }
    fn prompt_templates(&self, frame: gent_protocol::PromptTemplateFrame) -> Result<gent_protocol::PromptTemplateFrame, String> {
        use gent_protocol::PromptTemplateFrame;
        match frame {
            PromptTemplateFrame::Create { request_id, template } => Ok(PromptTemplateFrame::Created { request_id, template: self.prompt_templates.create(template)? }),
            PromptTemplateFrame::List { request_id } => Ok(PromptTemplateFrame::Templates { request_id, templates: self.prompt_templates.list()? }),
            PromptTemplateFrame::Get { request_id, template_id } => Ok(PromptTemplateFrame::Template { request_id, template: self.prompt_templates.get(&template_id)? }),
            PromptTemplateFrame::Delete { request_id, template_id } => { self.prompt_templates.delete(&template_id)?; Ok(PromptTemplateFrame::Deleted { request_id, template_id }) },
            PromptTemplateFrame::Render { request_id, render } => { let template_id = render.template_id.clone(); Ok(PromptTemplateFrame::Rendered { request_id, template_id, prompt: self.prompt_templates.render(render)? }) },
            _ => Err("prompt template response frames are server-only".into()),
        }
    }
    fn workspace_documents(&self, frame: gent_protocol::WorkspaceDocumentsFrame) -> Result<gent_protocol::WorkspaceDocumentsFrame, String> {
        use gent_protocol::WorkspaceDocumentsFrame;
        let WorkspaceDocumentsFrame::List { request_id, workspace_id } = frame else { return Err("workspace document response frames are server-only".into()); };
        let workspace = self.coordinator.workspace(&workspace_id).map_err(|error| error.to_string())?.ok_or_else(|| "workspace was not found".to_owned())?;
        let documents = crate::workspace_documents::discover(std::path::Path::new(&workspace.canonical_path))?;
        Ok(WorkspaceDocumentsFrame::Listed { request_id, workspace_id, documents })
    }
    fn workspace_git(&self, frame: gent_protocol::WorkspaceGitFrame) -> Result<gent_protocol::WorkspaceGitFrame, String> {
        crate::workspace_git_api::exchange(&self.coordinator, frame)
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
        let host_epoch = self
            .coordinator
            .status()
            .map_err(|error| error.to_string())?
            .host_epoch;
        attachment_api::handle(&self.attachments, host_epoch, frame)
    }
    fn local_models(&self, frame: LocalModelFrame) -> Result<LocalModelFrame, String> {
        self.local_model_frame(frame)
    }
    fn begin_local_model_download(
        &self,
        model_id: &str,
    ) -> Result<crate::standalone_authority_composition::LocalModelDownloadStart, String> {
        self.start_local_model_download(model_id)
    }
    fn finish_local_model_download(&self, model_id: &str) {
        self.complete_local_model_download(model_id);
    }
    fn publish_local_model_frame(&self, frame: LocalModelFrame) -> Result<(), String> {
        self.publish_local_model_frame(frame)
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
        self.coordinator.conversation_status(conversation_id).map_err(|error| error.to_string())
    }
    fn conversations(&self) -> Result<Vec<gent_types::ConversationListItem>, String> {
        self.coordinator.conversations().map_err(|error| error.to_string())
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
        self.coordinator.conversation_content(conversation_id, before.as_ref(), limit).map_err(|error| error.to_string())
    }
}
