//! Capability-gated activity endpoint; the observer daemon does not advertise it.

use gent_protocol::{
    CONVERSATION_ACTIVITY_CAPABILITY, ConversationActivityFrame, write_json_frame,
};
use gent_runtime::ConversationActivityRead;
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
        .any(|item| item == CONVERSATION_ACTIVITY_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(ConversationActivityFrame::Request {
        conversation_id,
        run_id,
        after_cursor,
    }) = serde_json::from_value(raw.clone())
    else {
        return Ok(false);
    };
    match runtime.conversation_activity(&conversation_id, &run_id, after_cursor) {
        Ok(ConversationActivityRead::Snapshot(activity)) => {
            write_json_frame(stream, &ConversationActivityFrame::Snapshot(activity)).await?;
        }
        Ok(ConversationActivityRead::Delta(activities)) => {
            write_json_frame(stream, &ConversationActivityFrame::Delta(activities)).await?;
        }
        Ok(ConversationActivityRead::Missing) => {
            write_error(stream, "notFound", "conversation activity does not exist").await?;
        }
        Ok(ConversationActivityRead::DeniedObserver) => {
            write_error(
                stream,
                "authorityUnavailable",
                "conversation activity is observer-disabled",
            )
            .await?;
        }
        Err(message) => write_error(stream, "invalidRequest", &message).await?,
    }
    Ok(true)
}
