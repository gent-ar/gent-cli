use super::RuntimeFacade;
use crate::agent_chat_intent_error::AgentChatIntentError;
use gent_protocol::AgentChatIntentFrame;
use gent_types::{AgentChatConversationId, AgentChatProvider, AgentChatRunId, HostEpoch};

impl RuntimeFacade {
    pub(super) fn targeted_steer_intent(
        &self,
        epoch: HostEpoch,
        frame: &AgentChatIntentFrame,
    ) -> Result<Option<AgentChatIntentFrame>, AgentChatIntentError> {
        if let AgentChatIntentFrame::SteerPromptForTurn(input) = frame {
            return self.atomic_steer_prompt(epoch, input).map(Some);
        }
        let AgentChatIntentFrame::SteerQueuedPromptForTurn {
            request_id,
            receipt_id,
            conversation_id,
            message_id,
            expected_run_id,
            expected_turn_id,
        } = frame
        else {
            return Ok(None);
        };
        let ingress = self
            .ordinary_prompt_ingress
            .as_ref()
            .ok_or("agent-chat provider lifecycle is not configured")?;
        let (receipt, run_id) = self
            .agent_chat_prompts
            .steer_queued_for_turn(
                receipt_id,
                epoch,
                conversation_id,
                message_id,
                expected_run_id,
                expected_turn_id,
            )?
            .ok_or("agent-chat authority is disabled")?;
        ingress.steer_run(
            self.run_provider(conversation_id, &run_id)?,
            crate::agent_chat_api::PromptWake {
                conversation_id: conversation_id.clone(),
                run_id,
                receipt_id: receipt.receipt_id.clone(),
                disposition: gent_types::AgentChatPromptDisposition::Queue,
            },
            Some(expected_turn_id),
        )?;
        Ok(Some(AgentChatIntentFrame::QueuedPromptSteered {
            request_id: request_id.clone(),
            receipt,
            conversation_id: conversation_id.clone(),
            message_id: message_id.clone(),
        }))
    }

    fn run_provider(
        &self,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<AgentChatProvider, AgentChatIntentError> {
        Ok(self
            .agent_chat_reads
            .as_ref()
            .ok_or("agent-chat reads are unavailable")?
            .run_selection(&conversation_id.0, &run_id.0)?
            .provider)
    }

    fn atomic_steer_prompt(
        &self,
        epoch: HostEpoch,
        input: &gent_protocol::AgentChatSteerPrompt,
    ) -> Result<AgentChatIntentFrame, AgentChatIntentError> {
        use crate::agent_chat_api::{PromptCommitWake, PromptWake};
        let ingress = self
            .ordinary_prompt_ingress
            .as_ref()
            .ok_or("agent-chat provider lifecycle is not configured")?;
        let _permit = ingress
            .acquire_prompt()
            .map_err(super::prompt_admission::prompt_admission_error)?;
        let result = self.agent_chat_prompts.submit_for_steer(
            &gent_runtime::AgentChatPromptRequest {
                request_id: input.request_id.clone(),
                receipt_id: input.receipt_id.clone(),
                host_epoch: epoch,
                conversation_id: input.conversation_id.clone(),
                disposition: gent_types::AgentChatPromptDisposition::Queue,
                text: input.text.clone(),
                attachment_ids: input.attachment_ids.clone(),
                tool_source_ids: input.tool_source_ids.clone(),
            },
            &input.expected_run_id,
            &input.expected_turn_id,
        )?;
        let gent_runtime::AgentChatPromptResult::Saved(saved) = result else {
            return Err("agent-chat authority is disabled".into());
        };
        let provider = self.run_provider(&input.conversation_id, &input.expected_run_id)?;
        let wake = PromptWake {
            conversation_id: input.conversation_id.clone(),
            run_id: saved.run_id.clone(),
            receipt_id: saved.receipt.receipt_id.clone(),
            disposition: saved.disposition,
        };
        let _ = ingress.clone().wake_after_prompt_commit(wake.clone());
        let _ = ingress.steer_run(
            provider,
            PromptWake {
                run_id: input.expected_run_id.clone(),
                ..wake
            },
            Some(&input.expected_turn_id),
        );
        Ok(AgentChatIntentFrame::Accepted {
            request_id: input.request_id.clone(),
            receipt: saved.receipt.clone(),
            conversation_id: input.conversation_id.clone(),
            run_id: saved.run_id.clone(),
            turn_id: saved.message.turn_id.clone(),
            message_id: saved.message.message_id.clone(),
            delivery: gent_types::AgentChatPromptDelivery::Queued,
        })
    }
}
