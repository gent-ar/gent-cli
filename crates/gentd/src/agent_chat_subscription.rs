//! Long-lived normalized transcript subscriptions backed only by the durable ledger.

use std::{io, time::Duration};

use gent_protocol::{
    AgentChatIntentFrame, AgentChatSubscriptionEnd, AgentChatTranscriptFrame, MAX_FRAME_BYTES,
    read_json_frame, write_json_frame,
};
use gent_types::{AgentChatConversationId, AgentChatRequestId, NormalizedTranscriptPage};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::api::RuntimeApi;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const PAGE_LIMIT: u16 = 100;

/// Serves one subscription until the client disconnects or the daemon can no longer read it.
pub(crate) async fn serve<S, R>(
    stream: S,
    runtime: R,
    request_id: AgentChatRequestId,
    conversation_id: AgentChatConversationId,
    after_cursor: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: TranscriptPort,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut cursor = after_cursor;
    loop {
        tokio::select! {
            input = read_json_frame::<_, Value>(&mut reader) => match input {
                Ok(_) => {
                    write_error(&mut writer, "invalidSubscriptionFrame", "only disconnect is valid after subscription").await?;
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(error) => return Err(Box::new(error)),
            },
            () = tokio::time::sleep(POLL_INTERVAL) => {
                if let Ok(events) = page(&runtime, &conversation_id, cursor) {
                    cursor = send_events(&mut writer, &request_id, cursor, events).await?;
                } else {
                    write_json_frame(&mut writer, &AgentChatIntentFrame::SubscriptionEnded {
                        request_id,
                        reason: AgentChatSubscriptionEnd::ServerClosing,
                    }).await?;
                    return Ok(());
                }
            }
        }
    }
}

pub(crate) trait TranscriptPort: Clone + Send + Sync + 'static {
    fn transcript(
        &self,
        frame: AgentChatTranscriptFrame,
    ) -> Result<AgentChatTranscriptFrame, String>;
}

impl<R: RuntimeApi> TranscriptPort for R {
    fn transcript(
        &self,
        frame: AgentChatTranscriptFrame,
    ) -> Result<AgentChatTranscriptFrame, String> {
        self.agent_chat_transcript(frame)
    }
}

fn page<R: TranscriptPort>(
    runtime: &R,
    conversation_id: &AgentChatConversationId,
    after_cursor: u64,
) -> Result<NormalizedTranscriptPage, String> {
    match runtime.transcript(AgentChatTranscriptFrame::PageRequest {
        conversation_id: conversation_id.0.clone(),
        after_cursor: Some(after_cursor),
        limit: PAGE_LIMIT,
    })? {
        AgentChatTranscriptFrame::Page(page) if page.conversation_id == conversation_id.0 => {
            Ok(page)
        }
        AgentChatTranscriptFrame::Page(_) => {
            Err("agent-chat transcript belongs to another conversation".into())
        }
        AgentChatTranscriptFrame::PageRequest { .. } => {
            Err("agent-chat runtime returned a transcript request".into())
        }
    }
}

async fn send_events<W>(
    writer: &mut W,
    request_id: &AgentChatRequestId,
    after_cursor: u64,
    page: NormalizedTranscriptPage,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>>
where
    W: AsyncWrite + Unpin,
{
    let mut cursor = after_cursor;
    for event in page.events {
        if event.cursor <= cursor {
            return Err("agent-chat transcript cursor is not strictly ascending".into());
        }
        let frame = AgentChatIntentFrame::SubscriptionEvent {
            request_id: request_id.clone(),
            event,
        };
        if serde_json::to_vec(&frame)?.len() > MAX_FRAME_BYTES {
            return Err("agent-chat transcript event exceeds IPC frame bound".into());
        }
        if let AgentChatIntentFrame::SubscriptionEvent { event, .. } = &frame {
            cursor = event.cursor;
        }
        write_json_frame(writer, &frame).await?;
    }
    Ok(cursor)
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use gent_protocol::{AgentChatIntentFrame, AgentChatTranscriptFrame, read_json_frame};
    use gent_types::{
        AgentChatConversationId, AgentChatRequestId, NormalizedTranscriptEvent,
        NormalizedTranscriptKind, NormalizedTranscriptPage,
    };
    use tokio::io::duplex;

    use super::{send_events, serve};
    use crate::agent_chat_subscription::TranscriptPort;

    #[derive(Clone)]
    struct TranscriptRuntime(Arc<Mutex<Vec<NormalizedTranscriptEvent>>>);

    impl TranscriptPort for TranscriptRuntime {
        fn transcript(
            &self,
            frame: AgentChatTranscriptFrame,
        ) -> Result<AgentChatTranscriptFrame, String> {
            let AgentChatTranscriptFrame::PageRequest {
                conversation_id,
                after_cursor,
                ..
            } = frame
            else {
                return Err("request expected".into());
            };
            let events = self
                .0
                .lock()
                .map_err(|_| "transcript lock poisoned")?
                .iter()
                .filter(|event| event.cursor > after_cursor.unwrap_or_default())
                .cloned()
                .collect();
            Ok(AgentChatTranscriptFrame::Page(NormalizedTranscriptPage {
                conversation_id,
                events,
                next_after_cursor: None,
            }))
        }
    }

    #[tokio::test]
    async fn subscription_replays_then_waits_for_later_durable_events() {
        let runtime = TranscriptRuntime(Arc::new(Mutex::new(vec![event(1)])));
        let source = runtime.clone();
        let (mut client, server) = duplex(32 * 1024);
        let task = tokio::spawn(serve(
            server,
            runtime,
            AgentChatRequestId("request-1".into()),
            AgentChatConversationId("conversation-1".into()),
            0,
        ));
        assert!(matches!(
            read_json_frame::<_, AgentChatIntentFrame>(&mut client).await.unwrap(),
            AgentChatIntentFrame::SubscriptionEvent { event, .. } if event.cursor == 1
        ));
        source.0.lock().unwrap().push(event(2));
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_millis(250), read_json_frame::<_, AgentChatIntentFrame>(&mut client)).await.unwrap().unwrap(),
            AgentChatIntentFrame::SubscriptionEvent { event, .. } if event.cursor == 2
        ));
        drop(client);
        assert!(task.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn oversized_events_are_rejected_before_writing() {
        let (mut reader, mut writer) = duplex(32 * 1024);
        let page = NormalizedTranscriptPage {
            conversation_id: "conversation-1".into(),
            events: vec![NormalizedTranscriptEvent {
                text: "x".repeat(gent_protocol::MAX_FRAME_BYTES),
                ..event(1)
            }],
            next_after_cursor: None,
        };
        assert!(
            send_events(&mut writer, &AgentChatRequestId("request".into()), 0, page)
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(20),
                read_json_frame::<_, AgentChatIntentFrame>(&mut reader)
            )
            .await
            .is_err()
        );
    }

    fn event(cursor: u64) -> NormalizedTranscriptEvent {
        NormalizedTranscriptEvent {
            cursor,
            event_id: format!("event-{cursor}"),
            turn_id: "turn-1".into(),
            run_id: "run-1".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "normalized".into(),
            is_partial: false,
        }
    }
}
