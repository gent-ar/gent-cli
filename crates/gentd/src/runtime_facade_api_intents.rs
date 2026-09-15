use gent_ports::ConversationLedger;
use gent_protocol::AgentChatIntentFrame;
use gent_types::HostEpoch;

use super::{RuntimeFacade, prompt_admission};
use crate::{agent_chat_api, agent_chat_intent_error::AgentChatIntentError};

impl RuntimeFacade {
    pub(super) fn exchange_agent_chat_intent(
        &self,
        frame: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        let host_epoch = self.host_epoch()?;
        let frame = self.with_default_selection(frame)?;
        self.validate_catalog_selection(&frame)?;
        let switched = self.switched_selection(&frame);
        if let AgentChatIntentFrame::SwitchSelection { parent_run_id, .. } = &frame {
            self.require_settled_selection_parent(&parent_run_id.0)?;
        }
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
        Ok(replies)
    }

    fn interrupt_intent(
        &self,
        host_epoch: HostEpoch,
        frame: AgentChatIntentFrame,
    ) -> Result<Option<AgentChatIntentFrame>, String> {
        let AgentChatIntentFrame::Interrupt {
            request_id,
            receipt_id,
            conversation_id,
            run_id,
        } = frame
        else {
            return Ok(None);
        };
        let ingress = self
            .ordinary_prompt_ingress
            .as_ref()
            .ok_or_else(|| "agent-chat provider lifecycle is not configured".to_owned())?;
        let selection = self
            .agent_chat_reads
            .as_ref()
            .ok_or_else(|| "agent-chat reads are unavailable".to_owned())?
            .run_selection(&conversation_id.0, &run_id.0)
            .map_err(|error| error.to_string())?;
        let receipt = self
            .coordinator
            .submit(&gent_types::Command {
                receipt_id: receipt_id.clone(),
                idempotency_key: format!("agent-chat-interrupt:{}", request_id.0),
                host_epoch,
                kind: "agentChatInterrupt".into(),
                payload: serde_json::json!({ "conversationId": conversation_id, "runId": run_id }),
            })
            .map_err(|error| error.to_string())?;
        if let Err(error) =
            self.goals
                .stop(&conversation_id, host_epoch, crate::startup::unix_seconds())
        {
            eprintln!(
                "goal stop for conversation {} was not recorded: {error}",
                conversation_id.0
            );
        }
        ingress.interrupt_run(selection.provider, &run_id.0)?;
        Ok(Some(AgentChatIntentFrame::Interrupted {
            request_id,
            receipt,
            conversation_id,
            run_id,
        }))
    }

    fn require_settled_selection_parent(&self, run_id: &str) -> Result<(), AgentChatIntentError> {
        let turns = self
            .transcript_import_ledger
            .list_run_turns(run_id)
            .map_err(|error| error.to_string())?;
        if turns.iter().any(|turn| !turn.phase.is_terminal()) {
            return Err(gent_types::AgentChatRejection::SelectionSwitchBlockedByActiveTurn.into());
        }
        Ok(())
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
                if ingress.steers_by_interrupt(provider)
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
