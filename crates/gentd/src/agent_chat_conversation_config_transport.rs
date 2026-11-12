//! Capability-gated local transport for per-conversation advanced launch configuration.

use gent_protocol::{
    AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY, AgentChatConversationConfigFrame, write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Decodes and responds to one conversation-config frame only after successful negotiation.
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
        .any(|capability| capability == AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<AgentChatConversationConfigFrame>(raw.clone()) else {
        return Ok(false);
    };
    if !matches!(
        frame,
        AgentChatConversationConfigFrame::Current { .. }
            | AgentChatConversationConfigFrame::Save { .. }
    ) {
        write_error(
            stream,
            "invalidAgentChatConversationConfig",
            "conversation config response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    match runtime.agent_chat_conversation_config(frame) {
        Ok(response) => write_json_frame(stream, &response).await?,
        Err(message) => {
            write_error(stream, "agentChatConversationConfigRejected", &message).await?;
        }
    }
    Ok(true)
}
