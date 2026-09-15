//! Capability-gated terminal following of one durable normalized turn.

use std::{
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};

use clap::Args;
use gent_protocol::{
    AGENT_CHAT_TURN_FOLLOW_CAPABILITY, AgentChatTurnFollowEnd, AgentChatTurnFollowFrame,
    read_json_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, NormalizedTranscriptEvent,
    TurnTerminal,
};
use serde::Serialize;
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

const MAX_RECONNECTS: u8 = 10;
const RECONNECT_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug, Args)]
pub(crate) struct FollowTurnArgs {
    #[arg(long, help = "Conversation that owns the turn")]
    conversation_id: String,
    #[arg(long, help = "Run that owns the turn")]
    run_id: String,
    #[arg(long, help = "Turn to follow")]
    turn_id: String,
    #[arg(
        long,
        default_value_t = 0,
        help = "Resume after this turn transcript cursor"
    )]
    after_cursor: u64,
    #[arg(
        long,
        default_value_t = 3,
        help = "Reconnect attempts after the stream closes (0-10)"
    )]
    reconnect_attempts: u8,
}

pub(crate) enum FollowItem<'a> {
    Event(&'a NormalizedTranscriptEvent),
    Terminal(&'a TurnTerminal),
}

pub(crate) fn print_json(item: FollowItem<'_>) -> io::Result<()> {
    match item {
        FollowItem::Event(event) => print(event),
        FollowItem::Terminal(terminal) => print(terminal),
    }
}

pub(crate) async fn run(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: FollowTurnArgs,
    sink: &mut impl FnMut(FollowItem<'_>) -> io::Result<()>,
) -> Result<TurnTerminal, Box<dyn std::error::Error>> {
    if args.reconnect_attempts > MAX_RECONNECTS {
        return Err(format!("--reconnect-attempts must not exceed {MAX_RECONNECTS}").into());
    }
    require_support(data_dir.clone(), no_autostart).await?;
    let mut cursor = args.after_cursor;
    for attempt in 0..=args.reconnect_attempts {
        match follow_once(data_dir.clone(), no_autostart, &args, &mut cursor, sink).await {
            Ok(FollowEnd::Terminal(terminal)) => return Ok(terminal),
            Ok(_) | Err(_) if attempt < args.reconnect_attempts => {
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
            Ok(FollowEnd::ResyncRequired) => {
                return Err(format!("turn stream requires resync after cursor {cursor}").into());
            }
            Ok(FollowEnd::ServerClosing) => {
                return Err(format!("turn stream closed after cursor {cursor}").into());
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the bounded reconnect loop always returns")
}

pub(crate) async fn follow_accepted_if_supported(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    run_id: String,
    turn_id: String,
    sink: &mut impl FnMut(FollowItem<'_>) -> io::Result<()>,
) -> Result<Option<TurnTerminal>, Box<dyn std::error::Error>> {
    let (_, capabilities) = connect_and_negotiate(data_dir.clone(), no_autostart).await?;
    if !supports(&capabilities.0) {
        return Ok(None);
    }
    run(
        data_dir,
        no_autostart,
        FollowTurnArgs {
            conversation_id,
            run_id,
            turn_id,
            after_cursor: 0,
            reconnect_attempts: 3,
        },
        sink,
    )
    .await
    .map(Some)
}

async fn require_support(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    supports(&capabilities.0)
        .then_some(())
        .ok_or_else(|| "daemon does not support exact turn follow; upgrade gentd".into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FollowEnd {
    Terminal(TurnTerminal),
    ResyncRequired,
    ServerClosing,
}

async fn follow_once(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: &FollowTurnArgs,
    cursor: &mut u64,
    sink: &mut impl FnMut(FollowItem<'_>) -> io::Result<()>,
) -> Result<FollowEnd, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !supports(&capabilities.0) {
        return Err("daemon does not support exact turn follow; upgrade gentd".into());
    }
    let request_id = AgentChatRequestId(uuid::Uuid::new_v4().to_string());
    write_json_frame(
        &mut stream,
        &AgentChatTurnFollowFrame::Follow {
            request_id: request_id.clone(),
            conversation_id: AgentChatConversationId(args.conversation_id.clone()),
            run_id: AgentChatRunId(args.run_id.clone()),
            turn_id: args.turn_id.clone(),
            after_cursor: *cursor,
        },
    )
    .await?;
    loop {
        let frame = decode(read_json_frame::<_, Value>(&mut stream).await?)?;
        match frame {
            AgentChatTurnFollowFrame::Event {
                request_id: reply,
                event,
            } => {
                accept_event(
                    &request_id,
                    &args.run_id,
                    &args.turn_id,
                    cursor,
                    &reply,
                    &event,
                )?;
                sink(FollowItem::Event(&event))?;
            }
            AgentChatTurnFollowFrame::Terminal {
                request_id: reply,
                terminal,
            } => {
                accept_terminal(&request_id, args, *cursor, &reply, &terminal)?;
                sink(FollowItem::Terminal(&terminal))?;
                return Ok(FollowEnd::Terminal(terminal));
            }
            AgentChatTurnFollowFrame::Ended {
                request_id: reply,
                reason,
            } => {
                if reply != request_id {
                    return Err("daemon ended another turn follow".into());
                }
                return Ok(match reason {
                    AgentChatTurnFollowEnd::ResyncRequired => FollowEnd::ResyncRequired,
                    AgentChatTurnFollowEnd::ServerClosing => FollowEnd::ServerClosing,
                });
            }
            AgentChatTurnFollowFrame::Follow { .. } => {
                return Err("daemon returned a client-only turn follow frame".into());
            }
        }
    }
}

fn supports(capabilities: &[String]) -> bool {
    capabilities
        .iter()
        .any(|capability| capability == AGENT_CHAT_TURN_FOLLOW_CAPABILITY)
}

fn decode(raw: Value) -> Result<AgentChatTurnFollowFrame, Box<dyn std::error::Error>> {
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.into());
    }
    serde_json::from_value(raw).map_err(|_| "daemon returned an invalid turn-follow frame".into())
}

fn accept_event(
    request: &AgentChatRequestId,
    run_id: &str,
    turn_id: &str,
    cursor: &mut u64,
    reply: &AgentChatRequestId,
    event: &NormalizedTranscriptEvent,
) -> Result<(), &'static str> {
    if reply != request || event.run_id != run_id || event.turn_id != turn_id {
        return Err("daemon delivered an event for another turn follow");
    }
    if event.cursor <= *cursor {
        return Err("daemon delivered a non-monotonic turn cursor");
    }
    *cursor = event.cursor;
    Ok(())
}

fn accept_terminal(
    request: &AgentChatRequestId,
    args: &FollowTurnArgs,
    cursor: u64,
    reply: &AgentChatRequestId,
    terminal: &TurnTerminal,
) -> Result<(), &'static str> {
    (reply == request
        && terminal.is_valid()
        && terminal.conversation_id == args.conversation_id
        && terminal.run_id == args.run_id
        && terminal.turn_id == args.turn_id
        && terminal.cursor == cursor)
        .then_some(())
        .ok_or("daemon returned an invalid turn terminal")
}

fn print(value: &impl Serialize) -> io::Result<()> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serde_json::to_writer(&mut writer, value).map_err(io::Error::other)?;
    writeln!(writer)?;
    writer.flush()
}

#[cfg(all(test, unix))]
#[path = "turn_follow_tests.rs"]
mod tests;
