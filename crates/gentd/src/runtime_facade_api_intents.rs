use gent_ports::AgentChatPromptDispatchLedger;
use gent_protocol::AgentChatIntentFrame;
use gent_types::HostEpoch;

use super::{RuntimeFacade, prompt_admission};
use crate::{
    agent_chat_api::{self, PromptCommitWake},
    agent_chat_intent_error::AgentChatIntentError,
};

#[path = "runtime_facade_interrupt.rs"]
mod interrupt;
#[path = "runtime_facade_provider_warm.rs"]
mod provider_warm;

impl RuntimeFacade {
    pub(super) fn exchange_agent_chat_intent(
        &self,
        frame: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        if let Some(reply) = self.provider_warm_intent(&frame)? {
            return Ok(vec![reply]);
        }
        if let Some(replies) = self.conversation_link_intent(&frame)? {
            return Ok(replies);
        }
        let host_epoch = self.host_epoch()?;
        if let Some(reply) = self.targeted_steer_intent(host_epoch, &frame)? {
            return Ok(vec![reply]);
        }
        let frame = self.with_default_selection(frame)?;
        let frame = self.fit_catalog_selection(frame)?;
        let switched = super::model_catalog::switched_selection(&frame);
        if let Some(reply) = self.interrupt_intent(host_epoch, frame.clone())? {
            return Ok(vec![reply]);
        }
        if let Some(reply) = self.queued_prompt_intent(host_epoch, frame.clone())? {
            return Ok(vec![reply]);
        }
        if let Some(replies) = self.session_continuation_intent(&frame)? {
            return Ok(replies);
        }
        if let AgentChatIntentFrame::ImportTranscript {
            request_id,
            conversation_id,
            run_id,
            entries,
        } = frame
        {
            return Ok(vec![self.import_transcript(
                request_id,
                conversation_id,
                run_id,
                &entries,
            )?]);
        }
        let replies = if let Some(ingress) = &self.ordinary_prompt_ingress {
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
        }?;
        self.remember_switched_selection(switched, &replies);
        for reply in &replies {
            if let AgentChatIntentFrame::Switched {
                conversation_id,
                run_id,
                receipt,
                ..
            } = reply
            {
                self.reap_superseded_warm_processes();
                self.wake_carried_prompts(conversation_id, run_id, &receipt.receipt_id);
            }
        }
        Ok(replies)
    }

    fn wake_carried_prompts(
        &self,
        conversation_id: &gent_types::AgentChatConversationId,
        run_id: &gent_types::AgentChatRunId,
        receipt_id: &gent_types::ReceiptId,
    ) {
        let Some(ingress) = &self.ordinary_prompt_ingress else {
            return;
        };
        if let Some(Ok(selection)) = self
            .agent_chat_reads
            .as_ref()
            .map(|reads| reads.run_selection(&conversation_id.0, &run_id.0))
            && let Err(error) = ingress.wake_run(
                selection.provider,
                agent_chat_api::PromptWake {
                    conversation_id: conversation_id.clone(),
                    run_id: run_id.clone(),
                    receipt_id: receipt_id.clone(),
                    disposition: gent_types::AgentChatPromptDisposition::Send,
                },
            )
        {
            eprintln!(
                "prompts carried to run {} were not woken: {error}",
                run_id.0
            );
        }
        let carried = match self
            .transcript_import_ledger
            .read_awaiting_agent_chat_prompts(Some(&run_id.0), "", 128)
        {
            Ok(carried) => carried,
            Err(error) => {
                eprintln!(
                    "prompts carried to run {} were not woken: {error}",
                    run_id.0
                );
                return;
            }
        };
        for saved in carried {
            let mut ingress = ingress.clone();
            if let Err(error) = ingress.wake_after_prompt_commit(agent_chat_api::PromptWake {
                conversation_id: conversation_id.clone(),
                run_id: run_id.clone(),
                receipt_id: saved.receipt.receipt_id,
                disposition: saved.disposition,
            }) {
                eprintln!("prompt carried to run {} was not woken: {error}", run_id.0);
            }
        }
    }

    fn queued_prompt_intent(
        &self,
        host_epoch: HostEpoch,
        frame: AgentChatIntentFrame,
    ) -> Result<Option<AgentChatIntentFrame>, AgentChatIntentError> {
        match frame {
            AgentChatIntentFrame::CancelQueuedPrompt {
                request_id,
                receipt_id,
                conversation_id,
                message_id,
            } => {
                let receipt = self
                    .agent_chat_prompts
                    .cancel_queued(&receipt_id, host_epoch, &conversation_id, &message_id)?
                    .ok_or("agent-chat authority is disabled")?;
                Ok(Some(AgentChatIntentFrame::QueuedPromptCanceled {
                    request_id,
                    receipt,
                    conversation_id,
                    message_id,
                }))
            }
            AgentChatIntentFrame::ArchiveConversation {
                request_id,
                receipt_id,
                conversation_id,
            } => {
                let receipt = self
                    .agent_chat_prompts
                    .archive(&receipt_id, host_epoch, &conversation_id)?
                    .ok_or("agent-chat authority is disabled")?;
                Ok(Some(AgentChatIntentFrame::ConversationArchived {
                    request_id,
                    receipt,
                    conversation_id,
                }))
            }
            AgentChatIntentFrame::SteerQueuedPrompt {
                request_id,
                receipt_id,
                conversation_id,
                message_id,
            } => {
                let ingress = self
                    .ordinary_prompt_ingress
                    .as_ref()
                    .ok_or("agent-chat provider lifecycle is not configured")?;
                let (receipt, run_id) = self
                    .agent_chat_prompts
                    .steer_queued(&receipt_id, host_epoch, &conversation_id, &message_id)?
                    .ok_or("agent-chat authority is disabled")?;
                let provider = self
                    .agent_chat_reads
                    .as_ref()
                    .ok_or("agent-chat reads are unavailable")?
                    .run_selection(&conversation_id.0, &run_id.0)?
                    .provider;
                if provider.steers_by_interrupt()
                    && !self.agent_chat_prompts.interrupt_active_turn_for_steer(
                        host_epoch,
                        &conversation_id,
                        &run_id,
                    )?
                {
                    return Ok(Some(AgentChatIntentFrame::QueuedPromptSteered {
                        request_id,
                        receipt,
                        conversation_id,
                        message_id,
                    }));
                }
                ingress.steer_run(
                    provider,
                    crate::agent_chat_api::PromptWake {
                        conversation_id: conversation_id.clone(),
                        run_id,
                        receipt_id: receipt.receipt_id.clone(),
                        disposition: gent_types::AgentChatPromptDisposition::Queue,
                    },
                    None,
                )?;
                Ok(Some(AgentChatIntentFrame::QueuedPromptSteered {
                    request_id,
                    receipt,
                    conversation_id,
                    message_id,
                }))
            }
            _ => Ok(None),
        }
    }
}
