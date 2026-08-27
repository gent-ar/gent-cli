//! Capability-gated local transport for durable per-turn file checkpoints.

use gent_protocol::{AGENT_CHAT_CHECKPOINT_CAPABILITY, AgentChatCheckpointFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Decodes and responds to one checkpoint frame only after successful negotiation.
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
        .any(|capability| capability == AGENT_CHAT_CHECKPOINT_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<AgentChatCheckpointFrame>(raw.clone()) else {
        return Ok(false);
    };
    if !matches!(
        frame,
        AgentChatCheckpointFrame::CaptureCheckpoint { .. }
            | AgentChatCheckpointFrame::ListCheckpoints { .. }
            | AgentChatCheckpointFrame::RestoreCheckpoint { .. }
    ) {
        write_error(
            stream,
            "invalidAgentChatCheckpoint",
            "checkpoint response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    match runtime.agent_chat_checkpoint(frame) {
        Ok(response) => write_json_frame(stream, &response).await?,
        Err(message) => write_error(stream, "agentChatCheckpointRejected", &message).await?,
    }
    Ok(true)
}
