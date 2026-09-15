use gent_protocol::{
    agent_chat_commands::{AGENT_CHAT_COMMANDS_CAPABILITY, AgentChatCommandFrame},
    write_json_frame,
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
        .any(|capability| capability == AGENT_CHAT_COMMANDS_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<AgentChatCommandFrame>(raw.clone()) else {
        return Ok(false);
    };
    if frame.validate().is_err() || !frame.is_client_request() {
        write_error(
            stream,
            "invalidAgentChatCommand",
            "agent-chat command request is invalid",
        )
        .await?;
        return Ok(true);
    }
    match runtime.agent_chat_command(frame) {
        Ok(reply) if reply.validate().is_ok() => write_json_frame(stream, &reply).await?,
        Ok(_) => {
            write_error(
                stream,
                "invalidAgentChatCommand",
                "agent-chat command runtime returned an invalid reply",
            )
            .await?;
        }
        Err(error) => write_error(stream, error.code, &error.message).await?,
    }
    Ok(true)
}

#[cfg(test)]
#[path = "agent_chat_command_transport_tests.rs"]
mod tests;
