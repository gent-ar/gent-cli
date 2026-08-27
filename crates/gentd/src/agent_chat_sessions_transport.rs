use gent_protocol::{AGENT_CHAT_SESSIONS_CAPABILITY, AgentChatSessionFrame, write_json_frame};
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
        .any(|item| item == AGENT_CHAT_SESSIONS_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<AgentChatSessionFrame>(raw.clone()) else {
        return Ok(false);
    };
    match runtime.agent_chat_sessions(frame) {
        Ok(reply) => write_json_frame(stream, &reply).await?,
        Err(error) => write_error(stream, "sessionRejected", &error).await?,
    }
    Ok(true)
}
