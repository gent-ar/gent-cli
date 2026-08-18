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

/// The exact durable boundary to use when attaching a replacement socket.
///
/// This contains only daemon-owned conversation identity and an acknowledged-or-applied
/// transcript cursor. It never retains provider-native session state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ControllerStreamResume {
    conversation_id: String,
    after_cursor: u64,
}

impl ControllerStreamResume {
    #[must_use]
    pub(crate) fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    #[must_use]
    pub(crate) fn after_cursor(&self) -> u64 {
        self.after_cursor
    }
}

/// A protocol or projection failure which must never be recovered from provider output.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ControllerStreamError {
    #[error("daemon did not negotiate selected-conversation controller streaming")]
    UnsupportedCapability,
    #[error("daemon sent a controller frame which is invalid in this direction")]
    UnexpectedFrame,
    #[error("daemon sent a controller update before its initial snapshot")]
    MissingSnapshot,
    #[error("daemon sent an initial snapshot after controller state was installed")]
    DuplicateSnapshot,
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
    attached_after_cursor: u64,
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
            attached_after_cursor: after_cursor,
            projection: None,
        })
    }

    /// Reads exactly one frame, replacing or strictly advancing local state before acknowledgement.
    pub(crate) async fn receive(&mut self) -> Result<ControllerStreamEvent, ControllerStreamError> {
        match read_json_frame::<_, AgentChatControllerStreamFrame>(&mut self.socket).await? {
            AgentChatControllerStreamFrame::Snapshot(snapshot) => {
                if self.projection.is_some() {
                    return Err(ControllerStreamError::DuplicateSnapshot);
                }
                self.install_projection(snapshot, self.attached_after_cursor)?;
                self.ack().await?;
                Ok(ControllerStreamEvent::ProjectionReplaced)
            }
            AgentChatControllerStreamFrame::Resync(snapshot) => {
                let current_cursor = self
                    .projection
                    .as_ref()
                    .ok_or(ControllerStreamError::MissingSnapshot)?
                    .transcript
                    .cursor();
                self.install_projection(snapshot, current_cursor)?;
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

    /// Returns the cursor boundary for a fresh socket after a close or resync request.
    ///
    /// The cursor advances before an acknowledgement is written: a reconnect must not
    /// duplicate an event already applied to the terminal, even if that write failed.
    #[must_use]
    pub(crate) fn resume(&self) -> ControllerStreamResume {
        ControllerStreamResume {
            conversation_id: self.conversation_id.clone(),
            after_cursor: self
                .projection
                .as_ref()
                .map_or(self.attached_after_cursor, |projection| {
                    projection.transcript.cursor()
                }),
        }
    }

    fn install_projection(
        &mut self,
        snapshot: AgentChatControllerSnapshot,
        minimum_cursor: u64,
    ) -> Result<(), ControllerStreamError> {
        if snapshot.cursor < minimum_cursor {
            return Err(ControllerStreamError::InvalidSnapshot);
        }
        self.projection = Some(projection(&self.conversation_id, snapshot)?);
        Ok(())
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

#[cfg(test)]
#[path = "controller_stream_resume_tests.rs"]
mod resume_tests;
