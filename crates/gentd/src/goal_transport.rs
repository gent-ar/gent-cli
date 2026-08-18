//! Capability-gated IPC for durable, provider-neutral `/goal` operations.

use gent_protocol::{GOAL_CAPABILITY, GoalFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Dispatches one goal request after an authority-gated capability negotiation.
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
    if !capabilities.0.iter().any(|item| item == GOAL_CAPABILITY) {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<GoalFrame>(raw.clone()) else {
        return Ok(false);
    };
    if frame.validate().is_err() || !client_request(&frame) {
        write_error(stream, "invalidGoal", "goal request is invalid").await?;
        return Ok(true);
    }
    match runtime.goal(frame) {
        Ok(reply) => write_json_frame(stream, &reply).await?,
        Err(message) => write_error(stream, "goalRejected", &message).await?,
    }
    Ok(true)
}

fn client_request(frame: &GoalFrame) -> bool {
    matches!(
        frame,
        GoalFrame::Create { .. }
            | GoalFrame::Transition { .. }
            | GoalFrame::Read { .. }
            | GoalFrame::List { .. }
    )
}

#[cfg(test)]
mod tests {
    use gent_protocol::GoalFrame;

    use super::client_request;

    #[test]
    fn replies_are_never_accepted_as_client_goal_requests() {
        assert!(!client_request(&GoalFrame::Goals {
            request_id: "request-1".into(),
            conversation_id: gent_types::AgentChatConversationId("conversation-1".into()),
            goals: Vec::new(),
        }));
    }
}
