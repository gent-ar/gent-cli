use gent_protocol::{
    AGENT_CHAT_PERMISSIONS_CAPABILITY, AgentChatPermissionFrame, write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

pub(crate) async fn dispatch<S, R>(
    stream: &mut S,
    runtime: &R,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == AGENT_CHAT_PERMISSIONS_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<AgentChatPermissionFrame>(raw.clone()) else {
        return Ok(false);
    };
    if matches!(
        frame,
        AgentChatPermissionFrame::Pending { .. } | AgentChatPermissionFrame::Accepted { .. }
    ) {
        write_error(
            stream,
            "invalidAgentChatPermission",
            "permission response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    let Some(port) = runtime.agent_chat_permission_port() else {
        write_error(
            stream,
            "agentChatPermissionUnavailable",
            "permission authority is unavailable",
        )
        .await?;
        return Ok(true);
    };
    dispatch_port(stream, port.as_ref(), raw).await
}

pub(crate) async fn dispatch_port<S, P>(
    stream: &mut S,
    port: &P,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    P: crate::agent_chat_permission_api::AgentChatPermissionPort + ?Sized,
{
    let Ok(frame) = serde_json::from_value::<AgentChatPermissionFrame>(raw.clone()) else {
        return Ok(false);
    };
    if matches!(
        frame,
        AgentChatPermissionFrame::Pending { .. } | AgentChatPermissionFrame::Accepted { .. }
    ) {
        write_error(
            stream,
            "invalidAgentChatPermission",
            "permission response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    match port.exchange(frame).await {
        Ok(response) => write_json_frame(stream, &response).await?,
        Err(message) => write_error(stream, "agentChatPermissionRejected", &message).await?,
    }
    Ok(true)
}
