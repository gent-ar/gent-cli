//! Capability-gated transport for reviewed-plan reads and user-owned decisions.

use gent_protocol::{REVIEWED_PLAN_CAPABILITY, ReviewedPlanFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

/// Dispatches one reviewed-plan exchange after negotiated authority is available.
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
        .any(|item| item == REVIEWED_PLAN_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<ReviewedPlanFrame>(raw.clone()) else {
        return Ok(false);
    };
    if frame.validate().is_err() || !client_request(&frame) {
        write_error(
            stream,
            "invalidReviewedPlan",
            "reviewed plan request is invalid",
        )
        .await?;
        return Ok(true);
    }
    match runtime.reviewed_plan(frame) {
        Ok(reply) => write_json_frame(stream, &reply).await?,
        Err(message) => write_error(stream, "reviewedPlanRejected", &message).await?,
    }
    Ok(true)
}

fn client_request(frame: &ReviewedPlanFrame) -> bool {
    matches!(
        frame,
        ReviewedPlanFrame::ReviewRead { .. }
            | ReviewedPlanFrame::StartImplementation { .. }
            | ReviewedPlanFrame::Reject { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::client_request;
    use gent_protocol::ReviewedPlanFrame;
    use gent_types::{AgentChatConversationId, ReviewedPlanId};

    #[test]
    fn review_reads_are_the_only_server_independent_plan_frames() {
        assert!(client_request(&ReviewedPlanFrame::ReviewRead {
            request_id: "request-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            plan_id: ReviewedPlanId("plan-1".into()),
        }));
    }
}
