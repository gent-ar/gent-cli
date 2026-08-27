use std::path::PathBuf;

use gent_protocol::{
    AGENT_CHAT_PERMISSIONS_CAPABILITY, AgentChatPermissionFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, PermissionDecisionRequest,
    PermissionDecisionResponse, ReceiptId,
};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn pending(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    run_id: String,
) -> Result<Option<PermissionDecisionRequest>, Box<dyn std::error::Error>> {
    let request_id = AgentChatRequestId(uuid::Uuid::new_v4().to_string());
    let request = AgentChatPermissionFrame::PendingRead {
        request_id: request_id.clone(),
        conversation_id: AgentChatConversationId(conversation_id),
        run_id: AgentChatRunId(run_id),
    };
    match exchange(data_dir, no_autostart, request).await? {
        AgentChatPermissionFrame::Pending {
            request_id: reply,
            request,
        } if reply == request_id => Ok(request),
        _ => Err("daemon returned an invalid pending permission response".into()),
    }
}

pub(crate) async fn respond(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    response: PermissionDecisionResponse,
) -> Result<(), Box<dyn std::error::Error>> {
    let request_id = AgentChatRequestId(uuid::Uuid::new_v4().to_string());
    let request = AgentChatPermissionFrame::Respond {
        request_id: request_id.clone(),
        receipt_id: ReceiptId(uuid::Uuid::new_v4().to_string()),
        response,
    };
    match exchange(data_dir, no_autostart, request).await? {
        AgentChatPermissionFrame::Accepted {
            request_id: reply, ..
        } if reply == request_id => Ok(()),
        _ => Err("daemon returned an invalid permission response".into()),
    }
}

pub(crate) async fn respond_json(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    response_json: String,
    receipt_id: Option<String>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let response = serde_json::from_str(&response_json)?;
    let request_id = AgentChatRequestId(uuid::Uuid::new_v4().to_string());
    let request = AgentChatPermissionFrame::Respond {
        request_id: request_id.clone(),
        receipt_id: ReceiptId(receipt_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string())),
        response,
    };
    let reply = exchange(data_dir, no_autostart, request).await?;
    if matches!(
        &reply,
        AgentChatPermissionFrame::Accepted {
            request_id: reply_id,
            ..
        } if reply_id == &request_id
    ) {
        Ok(serde_json::to_value(reply)?)
    } else {
        Err("daemon returned an invalid permission response".into())
    }
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: AgentChatPermissionFrame,
) -> Result<AgentChatPermissionFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == AGENT_CHAT_PERMISSIONS_CAPABILITY)
    {
        return Err("daemon does not support agent-chat permissions; upgrade gentd".into());
    }
    write_json_frame(&mut stream, &request).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    Ok(serde_json::from_value(raw)?)
}
