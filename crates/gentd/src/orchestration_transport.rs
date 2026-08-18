//! Reserved capability-gated transport for Gent-owned task-graph orchestration.

use gent_protocol::{ORCHESTRATION_CAPABILITY, OrchestrationFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Dispatches one typed orchestration frame only after capability negotiation.
///
/// An observer never advertises `orchestration-v1`, so it cannot reach the runtime port.
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
        .any(|item| item == ORCHESTRATION_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<OrchestrationFrame>(raw.clone()) else {
        return Ok(false);
    };
    if frame.validate().is_err() || !client_request(&frame) {
        write_error(
            stream,
            "invalidOrchestration",
            "orchestration request is invalid",
        )
        .await?;
        return Ok(true);
    }
    match runtime.orchestration(frame) {
        Ok(reply) => write_json_frame(stream, &reply).await?,
        Err(message) => write_error(stream, "orchestrationRejected", &message).await?,
    }
    Ok(true)
}

fn client_request(frame: &OrchestrationFrame) -> bool {
    matches!(
        frame,
        OrchestrationFrame::Fanout { .. }
            | OrchestrationFrame::CrossReview { .. }
            | OrchestrationFrame::GraphRead { .. }
    )
}

#[cfg(test)]
mod tests {
    use gent_protocol::OrchestrationFrame;

    use super::client_request;

    #[test]
    fn server_graph_frames_are_never_accepted_as_client_requests() {
        assert!(!client_request(&OrchestrationFrame::Graph {
            request_id: "request-1".into(),
            conversation_id: gent_types::AgentChatConversationId("conversation-1".into()),
            graph_id: "graph-1".into(),
            graph: None,
        }));
    }
}
