//! Unadvertised controller-stream transport seam for a future chat authority.
//!
//! This is intentionally not wired into the daemon listener. It only accepts client control
//! frames and emits server projections; providers and snapshot composition stay behind `RuntimeApi`.

use std::io;

use gent_protocol::{
    AgentChatControllerSnapshot, AgentChatControllerStreamEnd, AgentChatControllerStreamFrame,
    read_json_frame, write_json_frame,
};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{api::RuntimeApi, transport::write_error};

/// Serves one future controller stream after its separately negotiated capability.
///
/// `Snapshot`, `Delta`, `Resync`, and `End` are server-only: a client can attach or acknowledge a
/// cursor. This seam deliberately does not poll or launch any provider.
pub(crate) async fn serve<S, R>(
    stream: S,
    runtime: R,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: ControllerStreamPort,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let attach = if let AgentChatControllerStreamFrame::Attach {
        conversation_id,
        after_cursor,
    } = read_client_frame(&mut reader).await?
    {
        (conversation_id, after_cursor)
    } else {
        write_error(
            &mut writer,
            "invalidAgentChatControllerFrame",
            "controller stream must begin with an attach frame",
        )
        .await?;
        return Ok(());
    };
    let snapshot = match runtime.snapshot(&attach.0, attach.1) {
        Ok(snapshot) if valid_snapshot(&snapshot, &attach.0, attach.1) => snapshot,
        Ok(_) => {
            write_error(
                &mut writer,
                "invalidAgentChatControllerSnapshot",
                "controller runtime returned an invalid snapshot",
            )
            .await?;
            return Ok(());
        }
        Err(_) => {
            write_json_frame(
                &mut writer,
                &AgentChatControllerStreamFrame::End {
                    reason: AgentChatControllerStreamEnd::ConversationUnavailable,
                },
            )
            .await?;
            return Ok(());
        }
    };
    let mut acknowledged = attach.1;
    let visible_cursor = snapshot.cursor;
    write_json_frame(
        &mut writer,
        &AgentChatControllerStreamFrame::Snapshot(snapshot),
    )
    .await?;
    loop {
        match read_client_frame(&mut reader).await {
            Ok(AgentChatControllerStreamFrame::Ack { cursor })
                if cursor >= acknowledged && cursor <= visible_cursor =>
            {
                acknowledged = cursor;
            }
            Ok(AgentChatControllerStreamFrame::Ack { .. }) => {
                write_error(
                    &mut writer,
                    "invalidAgentChatControllerAck",
                    "controller acknowledgement is outside the visible cursor range",
                )
                .await?;
                return Ok(());
            }
            Ok(_) => {
                write_error(
                    &mut writer,
                    "invalidAgentChatControllerFrame",
                    "snapshot, delta, and resync frames are server-only",
                )
                .await?;
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(Box::new(error)),
        }
    }
}

/// Future authority boundary for a single normalized controller projection.
pub(crate) trait ControllerStreamPort: Clone + Send + Sync + 'static {
    fn snapshot(
        &self,
        conversation_id: &str,
        after_cursor: u64,
    ) -> Result<AgentChatControllerSnapshot, String>;
}

impl<R: RuntimeApi> ControllerStreamPort for R {
    fn snapshot(
        &self,
        conversation_id: &str,
        after_cursor: u64,
    ) -> Result<AgentChatControllerSnapshot, String> {
        self.agent_chat_controller_snapshot(conversation_id, after_cursor)
    }
}

fn valid_snapshot(
    snapshot: &AgentChatControllerSnapshot,
    conversation_id: &str,
    after: u64,
) -> bool {
    snapshot.conversation.summary.conversation_id == conversation_id
        && snapshot.transcript.conversation_id == conversation_id
        && snapshot.cursor >= after
        && snapshot
            .status
            .as_ref()
            .is_none_or(|status| status.conversation_id == conversation_id)
        && snapshot
            .transcript
            .events
            .windows(2)
            .all(|pair| pair[0].cursor < pair[1].cursor)
        && snapshot
            .transcript
            .events
            .last()
            .is_none_or(|event| event.cursor <= snapshot.cursor)
}

async fn read_client_frame<R>(reader: &mut R) -> io::Result<AgentChatControllerStreamFrame>
where
    R: AsyncRead + Unpin,
{
    read_json_frame::<_, Value>(reader)
        .await
        .and_then(|raw| serde_json::from_value(raw).map_err(io::Error::other))
}

#[cfg(test)]
#[path = "agent_chat_controller_transport_tests.rs"]
mod tests;
