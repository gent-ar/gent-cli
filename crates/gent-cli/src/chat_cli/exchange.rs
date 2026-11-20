use std::path::PathBuf;

use gent_protocol::{AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, write_json_frame};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

pub(super) async fn required(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: AgentChatIntentFrame,
    required_capabilities: &[&str],
) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    for required in
        std::iter::once(AGENT_CHAT_INTENTS_CAPABILITY).chain(required_capabilities.iter().copied())
    {
        if !capabilities.0.iter().any(|item| item == required) {
            return Err(format!("daemon does not support {required}; upgrade gentd").into());
        }
    }
    write_json_frame(&mut stream, &request).await?;
    let raw: Value = crate::local_ipc::read_reply(&mut stream).await?;
    let response =
        serde_json::from_value(raw).map_err(|_| "daemon did not return an agent-chat response")?;
    super::valid_reply(&request, &response)
        .then_some(response)
        .ok_or_else(|| {
            "daemon returned an agent-chat response with a different request or receipt".into()
        })
}

pub(crate) fn queue_valid_reply(
    request: &AgentChatIntentFrame,
    response: &AgentChatIntentFrame,
) -> Option<bool> {
    let (request_id, receipt_id, conversation_id, message_id, steer) = match request {
        AgentChatIntentFrame::SteerQueuedPromptForTurn {
            request_id,
            receipt_id,
            conversation_id,
            message_id,
            ..
        } => (request_id, receipt_id, conversation_id, message_id, true),
        AgentChatIntentFrame::CancelQueuedPrompt {
            request_id,
            receipt_id,
            conversation_id,
            message_id,
        } => (request_id, receipt_id, conversation_id, message_id, false),
        _ => return None,
    };
    match response {
        AgentChatIntentFrame::QueuedPromptSteered {
            request_id: reply,
            receipt,
            conversation_id: reply_conversation,
            message_id: reply_message,
        } => Some(
            steer
                && reply == request_id
                && receipt.receipt_id == *receipt_id
                && reply_conversation == conversation_id
                && reply_message == message_id,
        ),
        AgentChatIntentFrame::QueuedPromptCanceled {
            request_id: reply,
            receipt,
            conversation_id: reply_conversation,
            message_id: reply_message,
        } => Some(
            !steer
                && reply == request_id
                && receipt.receipt_id == *receipt_id
                && reply_conversation == conversation_id
                && reply_message == message_id,
        ),
        _ => Some(false),
    }
}
