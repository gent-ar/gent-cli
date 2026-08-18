//! Unadvertised controller-stream transport seam for a future chat authority.
//!
//! This is intentionally not wired into the daemon listener. It only accepts client control
//! frames and emits server projections; providers and snapshot composition stay behind `RuntimeApi`.

use std::{io, time::Duration};

use gent_protocol::{
    AgentChatControllerDelta, AgentChatControllerSnapshot, AgentChatControllerStreamEnd,
    AgentChatControllerStreamFrame, read_json_frame, write_json_frame,
};
use gent_runtime::AgentChatControllerDeltaPage;
use gent_types::HostEpoch;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{api::RuntimeApi, transport::write_error};

const POLL_INTERVAL: Duration = Duration::from_millis(50);

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
    let mut visible_cursor = snapshot.cursor;
    let host_epoch = snapshot.host_epoch;
    write_json_frame(
        &mut writer,
        &AgentChatControllerStreamFrame::Snapshot(snapshot),
    )
    .await?;
    loop {
        if acknowledged < visible_cursor {
            if !accept_ack(&mut reader, &mut writer, &mut acknowledged, visible_cursor).await? {
                return Ok(());
            }
            continue;
        }
        tokio::select! {
            input = read_client_frame(&mut reader) => {
                if !accept_frame(&mut writer, input, &mut acknowledged, visible_cursor).await? {
                    return Ok(());
                }
            }
            () = tokio::time::sleep(POLL_INTERVAL) => {
                if !send_delta(&mut writer, &runtime, &attach.0, &mut visible_cursor, host_epoch).await? {
                    return Ok(());
                }
            }
        }
    }
}

async fn accept_ack<R, W>(
    reader: &mut R,
    writer: &mut W,
    acknowledged: &mut u64,
    visible_cursor: u64,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    accept_frame(
        writer,
        read_client_frame(reader).await,
        acknowledged,
        visible_cursor,
    )
    .await
}

async fn accept_frame<W>(
    writer: &mut W,
    input: io::Result<AgentChatControllerStreamFrame>,
    acknowledged: &mut u64,
    visible_cursor: u64,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    W: AsyncWrite + Unpin,
{
    match input {
        Ok(AgentChatControllerStreamFrame::Ack { cursor })
            if cursor >= *acknowledged && cursor <= visible_cursor =>
        {
            *acknowledged = cursor;
            Ok(true)
        }
        Ok(AgentChatControllerStreamFrame::Ack { .. }) => {
            write_error(
                writer,
                "invalidAgentChatControllerAck",
                "controller acknowledgement is outside the visible cursor range",
            )
            .await?;
            Ok(false)
        }
        Ok(_) => {
            write_error(
                writer,
                "invalidAgentChatControllerFrame",
                "snapshot, delta, and resync frames are server-only",
            )
            .await?;
            Ok(false)
        }
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(Box::new(error)),
    }
}

async fn send_delta<W, R>(
    writer: &mut W,
    runtime: &R,
    conversation_id: &str,
    visible_cursor: &mut u64,
    host_epoch: HostEpoch,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    W: AsyncWrite + Unpin,
    R: ControllerStreamPort,
{
    let Ok(delta) = runtime.delta(conversation_id, *visible_cursor, host_epoch) else {
        write_end(writer, AgentChatControllerStreamEnd::ResyncRequired).await?;
        return Ok(false);
    };
    if !valid_delta(&delta, *visible_cursor, host_epoch) {
        write_end(writer, AgentChatControllerStreamEnd::ResyncRequired).await?;
        return Ok(false);
    }
    let Some(event) = delta.events.into_iter().next() else {
        return Ok(true);
    };
    *visible_cursor = event.cursor;
    write_json_frame(
        writer,
        &AgentChatControllerStreamFrame::Delta(AgentChatControllerDelta::Transcript {
            host_epoch,
            event,
        }),
    )
    .await?;
    Ok(true)
}

async fn write_end<W>(writer: &mut W, reason: AgentChatControllerStreamEnd) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    write_json_frame(writer, &AgentChatControllerStreamFrame::End { reason }).await
}

/// Future authority boundary for a single normalized controller projection.
pub(crate) trait ControllerStreamPort: Clone + Send + Sync + 'static {
    fn snapshot(
        &self,
        conversation_id: &str,
        after_cursor: u64,
    ) -> Result<AgentChatControllerSnapshot, String>;
    fn delta(
        &self,
        conversation_id: &str,
        after_cursor: u64,
        host_epoch: HostEpoch,
    ) -> Result<AgentChatControllerDeltaPage, String>;
}

impl<R: RuntimeApi> ControllerStreamPort for R {
    fn snapshot(
        &self,
        conversation_id: &str,
        after_cursor: u64,
    ) -> Result<AgentChatControllerSnapshot, String> {
        self.agent_chat_controller_snapshot(conversation_id, after_cursor)
    }

    fn delta(
        &self,
        conversation_id: &str,
        after_cursor: u64,
        host_epoch: HostEpoch,
    ) -> Result<AgentChatControllerDeltaPage, String> {
        self.agent_chat_controller_delta(conversation_id, after_cursor, host_epoch)
    }
}

fn valid_delta(
    delta: &AgentChatControllerDeltaPage,
    after_cursor: u64,
    host_epoch: HostEpoch,
) -> bool {
    if delta.host_epoch != host_epoch {
        return false;
    }
    let mut cursor = after_cursor;
    for event in &delta.events {
        if event.cursor <= cursor {
            return false;
        }
        cursor = event.cursor;
    }
    true
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
