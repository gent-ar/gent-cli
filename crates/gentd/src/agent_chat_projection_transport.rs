use std::{io, time::Duration};

use gent_protocol::{
    AGENT_CHAT_PROJECTION_CAPABILITY, AgentChatProjectionDelta, AgentChatProjectionFrame,
    ProjectionCursor, read_json_frame, write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{api::RuntimeApi, transport::write_error};

const POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) async fn serve<S, R>(
    stream: S,
    runtime: R,
    request_id: String,
    conversation_id: String,
    after_cursor: ProjectionCursor,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: RuntimeApi,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut cursor = after_cursor;
    let mut delay = Duration::ZERO;
    loop {
        tokio::select! {
            input = read_json_frame::<_, Value>(&mut reader) => match input {
                Ok(_) => return Err(Box::new(io::Error::other("only disconnect is valid after projection follow"))),
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(error) => return Err(Box::new(error)),
            },
            () = tokio::time::sleep(delay) => {
                let deltas = runtime
                    .agent_chat_projection_follow(&conversation_id, &cursor)
                    .map_err(io::Error::other)?;
                delay = if deltas.is_empty() { POLL_INTERVAL } else { Duration::ZERO };
                for delta in deltas {
                    let next = delta_cursor(&delta);
                    if next.value <= cursor.value {
                        return Err(Box::new(io::Error::other("projection cursor did not advance")));
                    }
                    cursor = next;
                    write_json_frame(&mut writer, &AgentChatProjectionFrame::Delta { request_id: request_id.clone(), delta }).await?;
                }
            }
        }
    }
}

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
        .any(|item| item == AGENT_CHAT_PROJECTION_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<AgentChatProjectionFrame>(raw.clone()) else {
        return Ok(false);
    };
    if !matches!(
        frame,
        AgentChatProjectionFrame::ConversationSnapshotRequest { .. }
    ) {
        return Ok(false);
    }
    match runtime.agent_chat_projection(frame) {
        Ok(reply @ AgentChatProjectionFrame::ConversationSnapshot { .. }) => {
            write_json_frame(stream, &reply).await?;
        }
        Ok(_) => {
            write_error(
                stream,
                "invalidAgentChatProjection",
                "agent-chat runtime returned an invalid projection frame",
            )
            .await?
        }
        Err(rejection) => write_error(stream, rejection.code, &rejection.message).await?,
    }
    Ok(true)
}

fn delta_cursor(delta: &AgentChatProjectionDelta) -> ProjectionCursor {
    match delta {
        AgentChatProjectionDelta::Transcript { cursor, .. }
        | AgentChatProjectionDelta::Activity { cursor, .. }
        | AgentChatProjectionDelta::Lifecycle { cursor, .. }
        | AgentChatProjectionDelta::Catalog { cursor, .. }
        | AgentChatProjectionDelta::ResyncRequired { cursor } => cursor.clone(),
    }
}

#[cfg(test)]
#[path = "agent_chat_projection_transport_tests.rs"]
mod tests;
