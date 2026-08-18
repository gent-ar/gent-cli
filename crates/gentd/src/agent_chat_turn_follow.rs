//! Read-only, cursor-resumable following of one durable agent-chat turn.

use std::{io, time::Duration};

use gent_protocol::{
    AgentChatTurnFollowEnd, AgentChatTurnFollowFrame, MAX_FRAME_BYTES, read_json_frame,
    write_json_frame,
};
use gent_runtime::{TurnFollowRead, TurnFollowRequest};
use gent_types::{AgentChatConversationId, AgentChatRequestId, AgentChatRunId, HostEpoch};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::api::RuntimeApi;

const PAGE_LIMIT: u16 = 100;
const MAX_PAGES_PER_POLL: usize = 4;
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Serves one exact turn from a read-only, epoch-fenced source until durable settlement.
///
/// Provider launch, native sessions, and raw provider output are deliberately absent from this
/// transport. The source already filters the exact conversation/run/turn tuple.
pub(crate) async fn serve<S, R>(
    stream: S,
    runtime: R,
    request_id: AgentChatRequestId,
    conversation_id: AgentChatConversationId,
    run_id: AgentChatRunId,
    turn_id: String,
    after_cursor: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: TurnFollowPort,
{
    let expected_epoch = runtime.host_epoch().map_err(io::Error::other)?;
    let scope = FollowScope {
        request_id,
        conversation_id,
        run_id,
        turn_id,
        expected_epoch,
    };
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut cursor = after_cursor;
    loop {
        tokio::select! {
            input = read_json_frame::<_, Value>(&mut reader) => match input {
                Ok(_) => {
                    write_error(&mut writer, "invalidTurnFollowFrame", "only disconnect is valid after turn follow").await?;
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(error) => return Err(Box::new(error)),
            },
            () = tokio::time::sleep(POLL_INTERVAL) => {
                match poll(&runtime, &scope, &mut cursor, &mut writer).await {
                    Ok(true) => return Ok(()),
                    Ok(false) => {}
                    Err(reason) => {
                        write_json_frame(&mut writer, &AgentChatTurnFollowFrame::Ended { request_id: scope.request_id.clone(), reason }).await?;
                        return Ok(());
                    }
                }
            }
        }
    }
}

struct FollowScope {
    request_id: AgentChatRequestId,
    conversation_id: AgentChatConversationId,
    run_id: AgentChatRunId,
    turn_id: String,
    expected_epoch: HostEpoch,
}

async fn poll<W, R>(
    runtime: &R,
    scope: &FollowScope,
    cursor: &mut u64,
    writer: &mut W,
) -> Result<bool, AgentChatTurnFollowEnd>
where
    W: AsyncWrite + Unpin,
    R: TurnFollowPort,
{
    for _ in 0..MAX_PAGES_PER_POLL {
        if runtime
            .host_epoch()
            .map_err(|_| AgentChatTurnFollowEnd::ServerClosing)?
            != scope.expected_epoch
        {
            return Err(AgentChatTurnFollowEnd::ResyncRequired);
        }
        let read = runtime
            .read(TurnFollowRequest {
                conversation_id: scope.conversation_id.0.clone(),
                run_id: scope.run_id.0.clone(),
                turn_id: scope.turn_id.clone(),
                after_cursor: *cursor,
                expected_host_epoch: scope.expected_epoch,
                limit: PAGE_LIMIT,
            })
            .map_err(|_| AgentChatTurnFollowEnd::ServerClosing)?;
        if read.host_epoch != scope.expected_epoch {
            return Err(AgentChatTurnFollowEnd::ResyncRequired);
        }
        let has_more = read.next_after_cursor.is_some();
        let advanced = send(read, cursor, writer, &scope.request_id)
            .await
            .map_err(|_| AgentChatTurnFollowEnd::ServerClosing)?;
        if advanced {
            return Ok(true);
        }
        if !has_more {
            return Ok(false);
        }
    }
    Ok(false)
}

async fn send<W>(
    read: TurnFollowRead,
    cursor: &mut u64,
    writer: &mut W,
    request_id: &AgentChatRequestId,
) -> io::Result<bool>
where
    W: AsyncWrite + Unpin,
{
    let before = *cursor;
    for event in read.events {
        if event.cursor <= *cursor {
            return Err(io::Error::other("turn follow cursor did not advance"));
        }
        let frame = AgentChatTurnFollowFrame::Event {
            request_id: request_id.clone(),
            event,
        };
        if serde_json::to_vec(&frame).map_err(io::Error::other)?.len() > MAX_FRAME_BYTES {
            return Err(io::Error::other(
                "turn follow event exceeds IPC frame bound",
            ));
        }
        if let AgentChatTurnFollowFrame::Event { event, .. } = &frame {
            *cursor = event.cursor;
        }
        write_json_frame(writer, &frame).await?;
    }
    if let Some(next) = read.next_after_cursor {
        if next < *cursor || (next == *cursor && *cursor == before) {
            return Err(io::Error::other("turn follow continuation did not advance"));
        }
        *cursor = next;
        return Ok(false);
    }
    if let Some(terminal) = read.terminal {
        if !terminal.is_valid() || terminal.cursor != *cursor {
            return Err(io::Error::other("turn follow terminal is invalid"));
        }
        write_json_frame(
            writer,
            &AgentChatTurnFollowFrame::Terminal {
                request_id: request_id.clone(),
                terminal,
            },
        )
        .await?;
        return Ok(true);
    }
    Ok(false)
}

async fn write_error<W>(writer: &mut W, code: &str, message: &str) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    gent_protocol::write_frame(
        writer,
        &gent_protocol::WireFrame::Error {
            code: code.into(),
            message: message.into(),
        },
    )
    .await
}

/// The daemon's read-only authority boundary for one turn-follow poll.
pub(crate) trait TurnFollowPort: Clone + Send + Sync + 'static {
    fn host_epoch(&self) -> Result<HostEpoch, String>;
    fn read(&self, request: TurnFollowRequest) -> Result<TurnFollowRead, String>;
}

impl<R: RuntimeApi> TurnFollowPort for R {
    fn host_epoch(&self) -> Result<HostEpoch, String> {
        self.status().map(|status| status.host_epoch)
    }

    fn read(&self, request: TurnFollowRequest) -> Result<TurnFollowRead, String> {
        self.agent_chat_turn_follow(request)
    }
}

#[cfg(test)]
#[path = "agent_chat_turn_follow_tests.rs"]
mod tests;
