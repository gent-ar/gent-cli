//! Typed selected-conversation stream client with no provider or rendering concerns.

use std::io;

use gent_protocol::{
    AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY, AgentChatControllerDelta, AgentChatControllerSnapshot,
    AgentChatControllerStreamEnd, AgentChatControllerStreamFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{AgentChatConversationDetail, ConversationStatus};
use tokio::io::{AsyncRead, AsyncWrite};

use super::chat_projection::{ChatProjection, ProjectionError};

/// The durable view replaced by controller snapshots and resyncs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ControllerProjection {
    conversation: AgentChatConversationDetail,
    status: Option<ConversationStatus>,
    transcript: ChatProjection,
}

impl ControllerProjection {
    #[must_use]
    pub(crate) fn transcript(&self) -> &ChatProjection {
        &self.transcript
    }
}

/// The next result from a controller stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ControllerStreamEvent {
    ProjectionReplaced,
    TranscriptApplied,
    ReconnectRequired,
    ConversationUnavailable,
}

/// A protocol or projection failure which must never be recovered from provider output.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ControllerStreamError {
    #[error("daemon did not negotiate selected-conversation controller streaming")]
    UnsupportedCapability,
    #[error("daemon sent a controller frame which is invalid in this direction")]
    UnexpectedFrame,
    #[error("daemon sent a transcript delta before a controller snapshot")]
    MissingSnapshot,
    #[error("daemon sent a controller snapshot for another conversation or cursor")]
    InvalidSnapshot,
    #[error(transparent)]
    Projection(#[from] ProjectionError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A capability-gated controller stream over an already-handshaken local socket.
#[derive(Debug)]
pub(crate) struct ControllerStream<S> {
    socket: S,
    conversation_id: String,
    projection: Option<ControllerProjection>,
}

impl<S> ControllerStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Checks the negotiated capability and attaches without any provider-native input.
    pub(crate) async fn attach(
        mut socket: S,
        capabilities: &[String],
        conversation_id: String,
        after_cursor: u64,
    ) -> Result<Self, ControllerStreamError> {
        if !supports_controller_stream(capabilities) {
            return Err(ControllerStreamError::UnsupportedCapability);
        }
        write_json_frame(
            &mut socket,
            &AgentChatControllerStreamFrame::Attach {
                conversation_id: conversation_id.clone(),
                after_cursor,
            },
        )
        .await?;
        Ok(Self {
            socket,
            conversation_id,
            projection: None,
        })
    }

    /// Reads exactly one frame, replacing or strictly advancing local state before acknowledgement.
    pub(crate) async fn receive(&mut self) -> Result<ControllerStreamEvent, ControllerStreamError> {
        match read_json_frame::<_, AgentChatControllerStreamFrame>(&mut self.socket).await? {
            AgentChatControllerStreamFrame::Snapshot(snapshot)
            | AgentChatControllerStreamFrame::Resync(snapshot) => {
                self.projection = Some(projection(&self.conversation_id, snapshot)?);
                self.ack().await?;
                Ok(ControllerStreamEvent::ProjectionReplaced)
            }
            AgentChatControllerStreamFrame::Delta(AgentChatControllerDelta::Transcript {
                host_epoch,
                event,
            }) => {
                let projection = self
                    .projection
                    .as_mut()
                    .ok_or(ControllerStreamError::MissingSnapshot)?;
                projection.transcript.apply(host_epoch, event)?;
                self.ack().await?;
                Ok(ControllerStreamEvent::TranscriptApplied)
            }
            AgentChatControllerStreamFrame::End { reason } => Ok(match reason {
                AgentChatControllerStreamEnd::ServerClosing
                | AgentChatControllerStreamEnd::ResyncRequired => {
                    ControllerStreamEvent::ReconnectRequired
                }
                AgentChatControllerStreamEnd::ConversationUnavailable => {
                    ControllerStreamEvent::ConversationUnavailable
                }
            }),
            AgentChatControllerStreamFrame::Attach { .. }
            | AgentChatControllerStreamFrame::Ack { .. } => {
                Err(ControllerStreamError::UnexpectedFrame)
            }
        }
    }

    #[must_use]
    pub(crate) fn projection(&self) -> Option<&ControllerProjection> {
        self.projection.as_ref()
    }

    async fn ack(&mut self) -> Result<(), ControllerStreamError> {
        let cursor = self
            .projection
            .as_ref()
            .expect("acknowledgements follow a successfully installed projection")
            .transcript
            .cursor();
        write_json_frame(
            &mut self.socket,
            &AgentChatControllerStreamFrame::Ack { cursor },
        )
        .await?;
        Ok(())
    }
}

pub(crate) fn supports_controller_stream(capabilities: &[String]) -> bool {
    capabilities
        .iter()
        .any(|capability| capability == AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY)
}

fn projection(
    conversation_id: &str,
    snapshot: AgentChatControllerSnapshot,
) -> Result<ControllerProjection, ControllerStreamError> {
    if snapshot.conversation.summary.conversation_id != conversation_id
        || snapshot
            .status
            .as_ref()
            .is_some_and(|status| status.conversation_id != conversation_id)
    {
        return Err(ControllerStreamError::InvalidSnapshot);
    }
    let initial_cursor = if snapshot.transcript.events.is_empty() {
        snapshot.cursor
    } else {
        0
    };
    let transcript = ChatProjection::from_page(
        conversation_id.into(),
        snapshot.host_epoch,
        initial_cursor,
        snapshot.transcript,
    )?;
    if transcript.cursor() != snapshot.cursor {
        return Err(ControllerStreamError::InvalidSnapshot);
    }
    Ok(ControllerProjection {
        conversation: snapshot.conversation,
        status: snapshot.status,
        transcript,
    })
}

#[cfg(test)]
#[path = "controller_stream_tests.rs"]
mod tests;
