use std::io;

use gent_protocol::{
    AGENT_CHAT_PROJECTION_CAPABILITY, AGENT_CHAT_PROJECTION_WINDOW_CAPABILITY,
    AgentChatProjectionDelta, AgentChatProjectionFrame, ProjectionCursor, read_json_frame,
    write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    api::RuntimeApi,
    transport::{off_runtime, write_error},
};

pub(crate) async fn serve<S, R>(
    stream: S,
    runtime: R,
    request_id: String,
    conversation_id: String,
    after_cursor: ProjectionCursor,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: ProjectionFollowPort,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut commits = runtime.commits();
    let mut catalogs = runtime.command_catalogs();
    catalogs.borrow_and_update();
    let mut cursor = after_cursor;
    loop {
        commits.borrow_and_update();
        let (conversation, after) = (conversation_id.clone(), cursor.clone());
        let deltas = match off_runtime(&runtime, move |runtime| {
            runtime.projection_deltas(&conversation, &after)
        })
        .await
        {
            Ok(Ok(deltas)) => deltas,
            Ok(Err(message)) => {
                return write_error(&mut writer, "projectionFollowFailed", &message).await;
            }
            Err(error) => {
                return write_error(&mut writer, "projectionFollowFailed", &error.to_string())
                    .await;
            }
        };
        let delivered = !deltas.is_empty();
        for delta in deltas {
            let next = delta_cursor(&delta);
            write_json_frame(
                &mut writer,
                &AgentChatProjectionFrame::Delta {
                    request_id: request_id.clone(),
                    delta,
                },
            )
            .await?;
            cursor = next;
        }
        if delivered {
            continue;
        }
        tokio::select! {
            input = read_json_frame::<_, Value>(&mut reader) => match input {
                Ok(_) => return write_error(&mut writer, "invalidProjectionFollowFrame", "only disconnect is valid after projection follow").await,
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(error) => return Err(Box::new(error)),
            },
            changed = commits.changed() => if changed.is_err() {
                return write_error(&mut writer, "projectionFollowFailed", "the ledger is closed").await;
            },
            Ok(()) = catalogs.changed() => {
                write_json_frame(
                    &mut writer,
                    &AgentChatProjectionFrame::Delta {
                        request_id: request_id.clone(),
                        delta: commands_listed(&cursor),
                    },
                )
                .await?;
            },
        }
    }
}

pub(crate) trait ProjectionFollowPort: Clone + Send + Sync + 'static {
    fn commits(&self) -> tokio::sync::watch::Receiver<u64>;
    fn command_catalogs(&self) -> tokio::sync::watch::Receiver<u64> {
        crate::runtime_facade::model_catalog::command_changes::subscribe()
    }
    fn projection_deltas(
        &self,
        conversation_id: &str,
        after_cursor: &ProjectionCursor,
    ) -> Result<Vec<AgentChatProjectionDelta>, String>;
}

impl<R: RuntimeApi> ProjectionFollowPort for R {
    fn commits(&self) -> tokio::sync::watch::Receiver<u64> {
        self.ledger_commits()
    }

    fn projection_deltas(
        &self,
        conversation_id: &str,
        after_cursor: &ProjectionCursor,
    ) -> Result<Vec<AgentChatProjectionDelta>, String> {
        self.agent_chat_projection_follow(conversation_id, after_cursor)
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
    let window = matches!(
        frame,
        AgentChatProjectionFrame::ConversationTurnCursorsRequest { .. }
            | AgentChatProjectionFrame::ConversationWindowRequest { .. }
    );
    let windows = capabilities
        .0
        .iter()
        .any(|item| item == AGENT_CHAT_PROJECTION_WINDOW_CAPABILITY);
    if !(window && windows
        || matches!(
            frame,
            AgentChatProjectionFrame::ConversationSnapshotRequest { .. }
        ))
    {
        return Ok(false);
    }
    match off_runtime(runtime, move |runtime| runtime.agent_chat_projection(frame)).await? {
        Ok(
            reply @ (AgentChatProjectionFrame::ConversationSnapshot { .. }
            | AgentChatProjectionFrame::ConversationTurnCursors { .. }
            | AgentChatProjectionFrame::ConversationWindow { .. }),
        ) => {
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

fn commands_listed(cursor: &ProjectionCursor) -> AgentChatProjectionDelta {
    AgentChatProjectionDelta::Catalog {
        cursor: cursor.clone(),
        catalog: gent_protocol::ProjectionCatalog {
            catalog_id: gent_protocol::COMMAND_CATALOG_ID.into(),
            selected_id: None,
            records: Vec::new(),
        },
    }
}

fn delta_cursor(delta: &AgentChatProjectionDelta) -> ProjectionCursor {
    match delta {
        AgentChatProjectionDelta::Transcript { cursor, .. }
        | AgentChatProjectionDelta::Activity { cursor, .. }
        | AgentChatProjectionDelta::Lifecycle { cursor, .. }
        | AgentChatProjectionDelta::Catalog { cursor, .. }
        | AgentChatProjectionDelta::Summary { cursor, .. }
        | AgentChatProjectionDelta::UnreadableRecord { cursor, .. } => cursor.clone(),
    }
}

#[cfg(test)]
#[path = "agent_chat_projection_transport_tests.rs"]
mod tests;
