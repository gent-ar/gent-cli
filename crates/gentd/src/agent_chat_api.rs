//! Maps supported agent-chat intent frames onto authority-gated runtime services.

use gent_protocol::AgentChatIntentFrame;
use gent_runtime::{
    AgentChatConversationRequest, AgentChatConversationResult, AgentChatConversationService,
    AgentChatPromptRequest, AgentChatPromptResult, AgentChatPromptService,
};
use gent_types::{AgentChatPromptDisposition, HostEpoch};

/// Handles the durable subset available before provider lifecycle composition.
pub(crate) fn exchange<L>(
    conversations: &AgentChatConversationService<L>,
    prompts: &AgentChatPromptService<L>,
    host_epoch: HostEpoch,
    frame: AgentChatIntentFrame,
) -> Result<Vec<AgentChatIntentFrame>, String>
where
    L: gent_ports::AgentChatLedger + gent_ports::AgentChatPromptLedger,
{
    match frame {
        AgentChatIntentFrame::CreateConversation {
            request_id,
            receipt_id,
            selection,
        } => create(conversations, host_epoch, request_id, receipt_id, selection),
        AgentChatIntentFrame::SendPrompt {
            request_id,
            receipt_id,
            conversation_id,
            text,
        } => prompt(
            prompts,
            host_epoch,
            request_id,
            receipt_id,
            conversation_id,
            text,
            AgentChatPromptDisposition::Send,
        ),
        AgentChatIntentFrame::QueuePrompt {
            request_id,
            receipt_id,
            conversation_id,
            text,
        } => prompt(
            prompts,
            host_epoch,
            request_id,
            receipt_id,
            conversation_id,
            text,
            AgentChatPromptDisposition::Queue,
        ),
        AgentChatIntentFrame::Interrupt { .. } | AgentChatIntentFrame::Decision { .. } => {
            Err("agent-chat provider lifecycle is not configured".into())
        }
        AgentChatIntentFrame::Subscribe { .. } => {
            Err("agent-chat transcript streaming is not configured".into())
        }
        _ => Err("agent-chat response frames are server-only".into()),
    }
}

fn create<L>(
    service: &AgentChatConversationService<L>,
    host_epoch: HostEpoch,
    request_id: gent_types::AgentChatRequestId,
    receipt_id: gent_types::ReceiptId,
    selection: gent_types::AgentChatSelection,
) -> Result<Vec<AgentChatIntentFrame>, String>
where
    L: gent_ports::AgentChatLedger,
{
    match service
        .create(&AgentChatConversationRequest {
            request_id: request_id.clone(),
            receipt_id,
            host_epoch,
            selection,
        })
        .map_err(|error| error.to_string())?
    {
        AgentChatConversationResult::Created(created) => Ok(vec![AgentChatIntentFrame::Created {
            request_id,
            receipt: created.receipt,
            conversation_id: created.conversation_id,
            run_id: created.run_id,
        }]),
        AgentChatConversationResult::DeniedObserver => {
            Err("agent-chat authority is disabled".into())
        }
    }
}

fn prompt<L>(
    service: &AgentChatPromptService<L>,
    host_epoch: HostEpoch,
    request_id: gent_types::AgentChatRequestId,
    receipt_id: gent_types::ReceiptId,
    conversation_id: gent_types::AgentChatConversationId,
    text: String,
    disposition: AgentChatPromptDisposition,
) -> Result<Vec<AgentChatIntentFrame>, String>
where
    L: gent_ports::AgentChatPromptLedger,
{
    match service
        .submit(&AgentChatPromptRequest {
            request_id: request_id.clone(),
            receipt_id,
            host_epoch,
            conversation_id,
            disposition,
            text,
        })
        .map_err(|error| error.to_string())?
    {
        AgentChatPromptResult::Saved(saved) => Ok(vec![AgentChatIntentFrame::Accepted {
            request_id,
            receipt: saved.receipt.clone(),
        }]),
        AgentChatPromptResult::DeniedObserver => Err("agent-chat authority is disabled".into()),
    }
}
