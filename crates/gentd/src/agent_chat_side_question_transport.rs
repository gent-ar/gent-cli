//! Capability-gated local transport for asking, cancelling, and reading side questions.

use gent_protocol::{AGENT_CHAT_SIDE_QUESTION_CAPABILITY, AgentChatSideQuestionFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Decodes and responds to one side-question frame only after successful negotiation.
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
        .any(|capability| capability == AGENT_CHAT_SIDE_QUESTION_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<AgentChatSideQuestionFrame>(raw.clone()) else {
        return Ok(false);
    };
    if !matches!(
        frame,
        AgentChatSideQuestionFrame::AskSideQuestion { .. }
            | AgentChatSideQuestionFrame::CancelSideQuestion { .. }
            | AgentChatSideQuestionFrame::ListSideQuestions { .. }
    ) {
        write_error(
            stream,
            "invalidAgentChatSideQuestion",
            "side question response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    match runtime.agent_chat_side_question(frame) {
        Ok(response) => write_json_frame(stream, &response).await?,
        Err(message) => write_error(stream, "agentChatSideQuestionRejected", &message).await?,
    }
    Ok(true)
}
