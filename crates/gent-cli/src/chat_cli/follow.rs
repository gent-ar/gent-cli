//! Long-lived, cursor-resumable reading of daemon-normalized chat events.

use std::{
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};

use clap::Args;
use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY, AgentChatIntentFrame,
    AgentChatSubscriptionEnd, WireFrame, read_json_frame, write_json_frame,
};
use gent_types::{AgentChatConversationId, AgentChatRequestId, NormalizedTranscriptEvent};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

const MAX_RECONNECTS: u8 = 10;

/// Streams one conversation's normalized events and resumes after a bounded number of disconnects.
#[derive(Debug, Args)]
pub(crate) struct FollowArgs {
    #[arg(long)]
    conversation_id: String,
    /// Resume strictly after this durable transcript cursor.
    #[arg(long, default_value_t = 0)]
    after_cursor: u64,
    /// Maximum reconnects after a closed daemon stream (0 through 10).
    #[arg(long, default_value_t = 3)]
    reconnect_attempts: u8,
}

pub(crate) async fn run(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: FollowArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    if args.reconnect_attempts > MAX_RECONNECTS {
        return Err(format!("--reconnect-attempts must not exceed {MAX_RECONNECTS}").into());
    }
    let mut cursor = args.after_cursor;
    for attempt in 0..=args.reconnect_attempts {
        match subscribe_once(
            data_dir.clone(),
            no_autostart,
            &args.conversation_id,
            &mut cursor,
        )
        .await
        {
            Ok(SubscriptionEnd::ResyncRequired) => {
                return Err("daemon requires a transcript resync; reopen the conversation".into());
            }
            Ok(SubscriptionEnd::ServerClosing) | Err(_) if attempt < args.reconnect_attempts => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(SubscriptionEnd::ServerClosing) => {
                return Err(format!(
                    "chat stream closed after cursor {cursor}; resume with --after-cursor {cursor}"
                )
                .into());
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the bounded reconnect loop always returns")
}

enum SubscriptionEnd {
    ResyncRequired,
    ServerClosing,
}

async fn subscribe_once(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &str,
    cursor: &mut u64,
) -> Result<SubscriptionEnd, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !supports_subscription(&capabilities.0) {
        return Err("daemon does not support normalized chat subscriptions; upgrade gentd".into());
    }
    let request_id = AgentChatRequestId(uuid::Uuid::new_v4().to_string());
    write_json_frame(
        &mut stream,
        &AgentChatIntentFrame::Subscribe {
            request_id: request_id.clone(),
            conversation_id: AgentChatConversationId(conversation_id.into()),
            after_cursor: *cursor,
        },
    )
    .await?;
    loop {
        let raw: Value = read_json_frame(&mut stream).await?;
        let frame = decode(raw)?;
        match frame {
            AgentChatIntentFrame::SubscriptionEvent {
                request_id: reply,
                event,
            } => {
                accept_event(&request_id, cursor, &reply, &event)?;
                print_event(&event)?;
            }
            AgentChatIntentFrame::SubscriptionEnded {
                request_id: reply,
                reason,
            } => {
                if reply != request_id {
                    return Err("daemon ended another chat subscription".into());
                }
                return Ok(match reason {
                    AgentChatSubscriptionEnd::ResyncRequired => SubscriptionEnd::ResyncRequired,
                    AgentChatSubscriptionEnd::ServerClosing => SubscriptionEnd::ServerClosing,
                });
            }
            _ => return Err("daemon returned a non-subscription chat frame".into()),
        }
    }
}

fn supports_subscription(capabilities: &[String]) -> bool {
    [
        AGENT_CHAT_INTENTS_CAPABILITY,
        AGENT_CHAT_TRANSCRIPT_CAPABILITY,
    ]
    .into_iter()
    .all(|required| capabilities.iter().any(|capability| capability == required))
}

fn decode(raw: Value) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    serde_json::from_value(raw)
        .map_err(|_| "daemon returned an invalid chat subscription frame".into())
}

fn accept_event(
    request: &AgentChatRequestId,
    cursor: &mut u64,
    reply: &AgentChatRequestId,
    event: &NormalizedTranscriptEvent,
) -> Result<(), &'static str> {
    if reply != request {
        return Err("daemon delivered an event for another chat subscription");
    }
    if event.cursor <= *cursor {
        return Err("daemon delivered a non-monotonic chat cursor");
    }
    *cursor = event.cursor;
    Ok(())
}

fn print_event(event: &NormalizedTranscriptEvent) -> io::Result<()> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serde_json::to_writer(&mut writer, event).map_err(io::Error::other)?;
    writeln!(writer)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use gent_types::{NormalizedTranscriptEvent, NormalizedTranscriptKind};

    use super::{
        AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY, AgentChatRequestId,
        FollowArgs, MAX_RECONNECTS, accept_event, supports_subscription,
    };

    #[test]
    fn subscriptions_require_both_intent_and_normalized_transcript_capabilities() {
        assert!(!supports_subscription(&[
            AGENT_CHAT_INTENTS_CAPABILITY.into()
        ]));
        assert!(!supports_subscription(&[
            AGENT_CHAT_TRANSCRIPT_CAPABILITY.into()
        ]));
        assert!(supports_subscription(&[
            AGENT_CHAT_INTENTS_CAPABILITY.into(),
            AGENT_CHAT_TRANSCRIPT_CAPABILITY.into(),
        ]));
    }

    fn event(cursor: u64) -> NormalizedTranscriptEvent {
        NormalizedTranscriptEvent {
            cursor,
            event_id: format!("event-{cursor}"),
            turn_id: "turn-1".into(),
            run_id: "run-1".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "normalized only".into(),
            is_partial: false,
        }
    }

    #[test]
    fn cursor_advances_only_for_the_matching_monotonic_subscription() {
        let request = AgentChatRequestId("request-1".into());
        let mut cursor = 3;
        assert!(accept_event(&request, &mut cursor, &request, &event(4)).is_ok());
        assert_eq!(cursor, 4);
        assert!(accept_event(&request, &mut cursor, &request, &event(4)).is_err());
        assert_eq!(cursor, 4);
        assert!(
            accept_event(
                &request,
                &mut cursor,
                &AgentChatRequestId("other-request".into()),
                &event(5),
            )
            .is_err()
        );
        assert_eq!(cursor, 4);
    }

    #[test]
    fn reconnect_budget_is_explicitly_bounded() {
        let args = FollowArgs {
            conversation_id: "conversation-1".into(),
            after_cursor: 0,
            reconnect_attempts: MAX_RECONNECTS,
        };
        assert_eq!(args.reconnect_attempts, 10);
    }
}
