use super::{PromptCommitWake, PromptInput, PromptWake};
use gent_protocol::AgentChatIntentFrame;
use gent_runtime::{AgentChatPromptRequest, AgentChatPromptResult, AgentChatPromptService};
use gent_types::AgentChatPromptDelivery;

pub(super) fn prompt<L, W>(
    service: &AgentChatPromptService<L>,
    host_epoch: gent_types::HostEpoch,
    input: PromptInput,
    wake: &mut W,
) -> Result<Vec<AgentChatIntentFrame>, String>
where
    L: gent_ports::AgentChatPromptLedger,
    W: PromptCommitWake,
{
    match service
        .submit(&AgentChatPromptRequest {
            request_id: input.request_id.clone(),
            receipt_id: input.receipt_id,
            host_epoch,
            conversation_id: input.conversation_id.clone(),
            disposition: input.disposition,
            text: input.text,
            attachment_ids: input.attachment_ids,
            tool_source_ids: input.tool_source_ids,
        })
        .map_err(|error| error.to_string())?
    {
        AgentChatPromptResult::Saved(saved) => {
            let should_notify = saved.delivery == AgentChatPromptDelivery::AwaitingProvider
                || (saved.delivery == AgentChatPromptDelivery::AwaitingReadiness
                    && wake.handles_awaiting_readiness());
            let mut delivery = saved.delivery;
            if should_notify {
                if wake
                    .wake_after_prompt_commit(PromptWake {
                        conversation_id: input.conversation_id.clone(),
                        run_id: saved.run_id.clone(),
                        receipt_id: saved.receipt.receipt_id.clone(),
                        disposition: saved.disposition,
                    })
                    .is_err()
                {
                    delivery = AgentChatPromptDelivery::Queued;
                }
            }
            Ok(vec![AgentChatIntentFrame::Accepted {
                request_id: input.request_id,
                receipt: saved.receipt.clone(),
                conversation_id: input.conversation_id,
                run_id: saved.run_id.clone(),
                turn_id: saved.message.turn_id.clone(),
                delivery,
            }])
        }
        AgentChatPromptResult::DeniedObserver => Err("agent-chat authority is disabled".into()),
    }
}
