use std::path::PathBuf;

use gent_protocol::{
    AgentChatIntentFrame, conversation_links::CONVERSATION_LINKS_CAPABILITY, read_json_frame,
    write_json_frame,
};
use gent_types::{AgentChatConversationId, AgentChatRequestId, ReceiptId};
use serde_json::Value;

use crate::local_ipc::{LocalStream, connect_and_negotiate};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CallerConversation(pub(crate) String);

pub(crate) async fn create(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    caller: &CallerConversation,
    label: String,
    prompt: Option<String>,
    workspace_path: Option<String>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let request_id = request_id();
    exchange(
        data_dir,
        no_autostart,
        AgentChatIntentFrame::CreateLinkedConversation {
            receipt_id: ReceiptId(format!("chat-create-{request_id}")),
            request_id: AgentChatRequestId(request_id),
            parent_conversation_id: AgentChatConversationId(caller.0.clone()),
            workspace_path,
            selection: None,
            label,
            prompt,
            origin_tool_use_id: None,
        },
    )
    .await
}

pub(crate) async fn send(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    caller: &CallerConversation,
    target_conversation_id: String,
    message: String,
) -> Result<Value, Box<dyn std::error::Error>> {
    let request_id = request_id();
    exchange(
        data_dir,
        no_autostart,
        AgentChatIntentFrame::SendToConversation {
            receipt_id: ReceiptId(format!("chat-send-{request_id}")),
            request_id: AgentChatRequestId(request_id),
            from_conversation_id: AgentChatConversationId(caller.0.clone()),
            target_conversation_id: AgentChatConversationId(target_conversation_id),
            message,
        },
    )
    .await
}

pub(crate) async fn wait(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    caller: &CallerConversation,
    conversation_ids: Vec<String>,
    timeout_seconds: u32,
) -> Result<Value, Box<dyn std::error::Error>> {
    exchange(
        data_dir,
        no_autostart,
        AgentChatIntentFrame::WaitForConversations {
            request_id: AgentChatRequestId(request_id()),
            from_conversation_id: AgentChatConversationId(caller.0.clone()),
            conversation_ids: conversation_ids
                .into_iter()
                .map(AgentChatConversationId)
                .collect(),
            timeout_seconds,
        },
    )
    .await
}

pub(crate) async fn list(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    caller: &CallerConversation,
) -> Result<Value, Box<dyn std::error::Error>> {
    exchange(
        data_dir,
        no_autostart,
        AgentChatIntentFrame::ListLinkedConversations {
            request_id: AgentChatRequestId(request_id()),
            conversation_id: AgentChatConversationId(caller.0.clone()),
        },
    )
    .await
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: AgentChatIntentFrame,
) -> Result<Value, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    capabilities
        .0
        .iter()
        .any(|item| item == CONVERSATION_LINKS_CAPABILITY)
        .then_some(())
        .ok_or("conversation links are unavailable from this gentd; no conversation was changed")?;
    reply(&mut stream, request).await
}

async fn reply(
    stream: &mut LocalStream,
    request: AgentChatIntentFrame,
) -> Result<Value, Box<dyn std::error::Error>> {
    write_json_frame(stream, &request).await?;
    let raw: Value = read_json_frame(stream).await?;
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.into());
    }
    let response: AgentChatIntentFrame = serde_json::from_value(raw.clone())
        .map_err(|_| "gentd did not return a conversation link response")?;
    match response {
        AgentChatIntentFrame::LinkedConversationCreated {
            conversation_id,
            label,
            ..
        } => Ok(serde_json::json!({
            "conversationId": conversation_id.0,
            "label": label,
            "openable": true,
        })),
        AgentChatIntentFrame::ConversationMessageDelivered {
            target_conversation_id,
            delivery,
            message_id,
            ..
        } => Ok(serde_json::json!({
            "conversationId": target_conversation_id.0,
            "delivery": delivery,
            "messageId": message_id,
        })),
        AgentChatIntentFrame::ConversationWaitSettled {
            results, timed_out, ..
        } => Ok(serde_json::json!({ "results": results, "timedOut": timed_out })),
        AgentChatIntentFrame::ConversationLinks { links, .. } => Ok(serde_json::json!(links)),
        _ => Err("gentd returned a conversation link response with a different identity".into()),
    }
}

fn request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
#[path = "chat_links_cli_tests.rs"]
mod tests;
