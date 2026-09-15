use gent_ports::{
    ClaurstPromptAttachment, ClaurstSourceId, ClaurstStartRequest, PrivateClaurstBridge, RunLease,
    RunLeaseClaim,
};
use gent_runtime::AgentChatPromptDispatchResult;
use gent_types::{
    AgentChatPromptSaved, AgentChatProvider, AgentChatRunContext, ConversationActivityFact,
    ConversationActivityScope, FrozenConversationContext, NormalizedProviderEvent,
    NormalizedSessionBatch, NormalizedSessionLifecycle,
};
use sha2::{Digest, Sha256};

use super::compaction::{Admission, Work};
use super::{ActivePrompt, ClaurstLifecycleLedger, ClaurstPromptLifecycle, display};
use crate::claurst_runtime_factory::ClaurstRuntimeFactory;

impl<L, B, F> ClaurstPromptLifecycle<L, B, F>
where
    L: ClaurstLifecycleLedger,
    B: PrivateClaurstBridge,
    F: ClaurstRuntimeFactory,
{
    pub(super) async fn claim_and_start(&mut self) -> Result<bool, String> {
        match self
            .dispatches
            .claim(
                &self.coordinator_id,
                self.host_epoch,
                AgentChatProvider::Claurst,
            )
            .map_err(display)?
        {
            AgentChatPromptDispatchResult::DeniedObserver
            | AgentChatPromptDispatchResult::Empty => Ok(false),
            AgentChatPromptDispatchResult::Claimed(saved) => self.start_claimed(*saved).await,
        }
    }

    async fn start_claimed(&mut self, saved: AgentChatPromptSaved) -> Result<bool, String> {
        if !self.active.is_empty() {
            self.release_claim(&saved)?;
            return Ok(true);
        }
        let lease = RunLease {
            run_id: saved.run_id.0.clone(),
            coordinator_id: self.coordinator_id.clone(),
            host_epoch: self.host_epoch,
        };
        if matches!(
            self.lease_coordinator
                .claim_run_lease(&lease)
                .map_err(display)?,
            RunLeaseClaim::Contended(_)
        ) {
            self.release_claim(&saved)?;
            return Err("standalone Claurst run lease is owned by another coordinator".into());
        }
        let attachment_metadata = self
            .ledger
            .turn_attachments(&saved.message.turn_id)
            .map_err(display)?;
        let attachments = match self.runtime.prompt_attachments(&attachment_metadata).await {
            Ok(attachments) => attachments,
            Err(error) => return self.fail_prelaunch(&saved, &error),
        };
        let source_id = source_id(&saved);
        let (request, boundary) = self.start_request(&saved, &source_id, &attachments)?;
        if let Err(error) = self.runtime.ensure_for_prompt(&saved).await {
            return self.fail_prelaunch(&saved, &error);
        }
        let summarizer = self.runtime.context_summarizer().await;
        let fallback = match self.admission(
            &saved,
            &boundary,
            &request,
            attachment_metadata.is_empty(),
            summarizer.as_ref(),
        )? {
            Admission::Reject(error) => return self.fail_prelaunch(&saved, &error),
            Admission::Prompt { fallback } => fallback,
            Admission::Compact { plan, then_start } => {
                let request = then_start.then(|| Box::new(request));
                return self
                    .begin_compaction(saved, source_id, *plan, request, summarizer)
                    .await;
            }
        };
        if let Err(error) = self.dispatches.begin_launch(
            &saved.message.message_id,
            &self.coordinator_id,
            self.host_epoch,
        ) {
            return Err(self.stop_failed_runtime(&saved, display(error)).await);
        }
        if let Err(error) = self.ingress.start(request, self.host_epoch).await {
            let error = display(error);
            let failure = self.stop_failed_runtime(&saved, error.clone()).await;
            self.record_notice(
                &saved,
                "claurst-start-failed",
                format!("Claurst could not start: {error}"),
            )?;
            self.dispatches
                .mark_unprovable(
                    &saved.message.message_id,
                    &self.coordinator_id,
                    self.host_epoch,
                )
                .map_err(display)?;
            eprintln!("Claurst prompt could not start: {failure}");
            return Ok(true);
        }
        if let Err(error) = self.dispatches.confirm_started(
            &saved.message.message_id,
            &self.coordinator_id,
            self.host_epoch,
        ) {
            return Err(self.stop_failed_runtime(&saved, display(error)).await);
        }
        if let Err(error) = self.record_turn_started(&saved, &source_id) {
            return Err(self.stop_failed_runtime(&saved, error).await);
        }
        if fallback {
            self.record_notice(
                &saved,
                "context-compaction-fallback",
                gent_types::CONTEXT_COMPACTION_FALLBACK_NOTICE.into(),
            )?;
        }
        self.active.insert(
            source_id,
            ActivePrompt {
                saved,
                work: Work::Provider,
            },
        );
        Ok(true)
    }

    fn release_claim(&self, saved: &AgentChatPromptSaved) -> Result<(), String> {
        self.dispatches
            .release_claim(
                &saved.message.message_id,
                &self.coordinator_id,
                self.host_epoch,
            )
            .map_err(display)
    }

    fn fail_prelaunch(&self, saved: &AgentChatPromptSaved, error: &str) -> Result<bool, String> {
        self.dispatches
            .fail_prelaunch(
                &saved.message.message_id,
                &self.coordinator_id,
                self.host_epoch,
                error,
            )
            .map_err(display)?;
        Ok(true)
    }

    fn start_request(
        &self,
        saved: &AgentChatPromptSaved,
        source_id: &ClaurstSourceId,
        attachments: &[ClaurstPromptAttachment],
    ) -> Result<(ClaurstStartRequest, AgentChatRunContext), String> {
        let conversation_config = self
            .ledger
            .current_conversation_config(&saved.message.conversation_id)
            .map_err(display)?;
        let message_text = match conversation_config
            .as_ref()
            .filter(|config| config.append_system_prompt)
            .and_then(|config| config.system_prompt.as_deref())
        {
            Some(configured) => format!("{configured}\n\n{}", saved.message.text),
            None => saved.message.text.clone(),
        };
        let prompt =
            crate::provider_attachments::claurst_prompt_with_files(&message_text, attachments);
        let (context, boundary) = self.prompt_context(saved)?;
        Ok((
            ClaurstStartRequest {
                run_id: saved.run_id.0.clone(),
                source_id: source_id.clone(),
                turn_id: saved.message.turn_id.clone(),
                prompt,
                context,
                attachments: crate::provider_attachments::claurst_images(attachments),
                goal: None,
            },
            boundary,
        ))
    }

    pub(super) fn prompt_context(
        &self,
        saved: &AgentChatPromptSaved,
    ) -> Result<(FrozenConversationContext, AgentChatRunContext), String> {
        let boundary = self
            .contexts
            .resolve(
                &gent_types::AgentChatConversationId(saved.message.conversation_id.clone()),
                &saved.run_id,
            )
            .map_err(display)?;
        let context = match self
            .artifacts
            .project_run_summarized(&boundary, &saved.message.message_id)
            .map_err(display)?
        {
            Some(summarized) => summarized,
            None => self
                .artifacts
                .project_run_before_message(&boundary, &saved.message.message_id)
                .map_err(display)?,
        };
        Ok((context, boundary))
    }

    pub(super) fn record_turn_started(
        &self,
        saved: &AgentChatPromptSaved,
        source_id: &ClaurstSourceId,
    ) -> Result<(), String> {
        let lifecycle = NormalizedSessionLifecycle::Event {
            event: NormalizedProviderEvent::TurnStarted {
                turn_id: saved.message.turn_id.clone(),
            },
        };
        self.ledger
            .append_normalized_session_batch(&NormalizedSessionBatch {
                coordinator_id: self.coordinator_id.clone(),
                conversation_id: saved.message.conversation_id.clone(),
                run_id: saved.run_id.0.clone(),
                turn_id: saved.message.turn_id.clone(),
                host_epoch: self.host_epoch,
                lifecycle_event_id: format!("{}:turn-started", source_id.0),
                lifecycle,
                transcript: None,
                activity_event_id: Some(format!("{}:turn-started-activity", source_id.0)),
                activity: Some(ConversationActivityFact::TurnStarted {
                    scope: ConversationActivityScope {
                        conversation_id: saved.message.conversation_id.clone(),
                        run_id: saved.run_id.0.clone(),
                        turn_id: saved.message.turn_id.clone(),
                        host_epoch: self.host_epoch,
                        cursor: 0,
                    },
                }),
            })
            .map(|_| ())
            .map_err(display)
    }
}

fn source_id(saved: &AgentChatPromptSaved) -> ClaurstSourceId {
    let material = format!(
        "{}\0{}\0{}",
        saved.run_id.0, saved.message.turn_id, saved.message.message_id
    );
    ClaurstSourceId(format!(
        "gent-{}",
        hex::encode(Sha256::digest(material.as_bytes()))
    ))
}
