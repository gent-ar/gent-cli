use std::io;

use gent_protocol::{
    AgentChatIntentFrame, AgentChatSubscriptionEnd, AgentChatTranscriptFrame, MAX_FRAME_BYTES,
    read_json_frame, write_json_frame,
};
use gent_types::{AgentChatConversationId, AgentChatRequestId, NormalizedTranscriptPage};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{
    api::RuntimeApi,
    transport::{off_runtime, write_error},
};

const PAGE_LIMIT: u16 = 100;

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
    let mut commits = runtime.commits();
    let mut cursor = after_cursor;
    loop {
        commits.borrow_and_update();
        let conversation = conversation_id.clone();
        let read = off_runtime(&runtime, move |runtime| {
            page(&runtime, &conversation, cursor)
        })
        .await;
        let Ok(Ok(events)) = read else {
            return end(&mut writer, request_id).await;
        };
        let delivered = !events.events.is_empty();
        cursor = match send_events(&mut writer, &request_id, cursor, events).await {
            Ok(cursor) => cursor,
            Err(error) => {
                return write_error(&mut writer, "subscriptionFailed", &error.to_string()).await;
            }
        };
        if delivered {
            continue;
        }
        tokio::select! {
            input = read_json_frame::<_, Value>(&mut reader) => match input {
                Ok(_) => {
                    write_error(&mut writer, "invalidSubscriptionFrame", "only disconnect is valid after subscription").await?;
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(error) => return Err(Box::new(error)),
            },
            changed = commits.changed() => if changed.is_err() {
                return end(&mut writer, request_id).await;
            },
        }
    }
}

async fn end<W>(
    writer: &mut W,
    request_id: AgentChatRequestId,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    W: AsyncWrite + Unpin,
{
    write_json_frame(
        writer,
        &AgentChatIntentFrame::SubscriptionEnded {
            request_id,
            reason: AgentChatSubscriptionEnd::ServerClosing,
        },
    )
    .await?;
    Ok(())
}

pub(crate) trait TranscriptPort: Clone + Send + Sync + 'static {
    fn commits(&self) -> tokio::sync::watch::Receiver<u64>;
    fn transcript(
        &self,
        frame: AgentChatTranscriptFrame,
    ) -> Result<AgentChatTranscriptFrame, String>;
}

impl<R: RuntimeApi> TranscriptPort for R {
    fn commits(&self) -> tokio::sync::watch::Receiver<u64> {
        self.ledger_commits()
    }

    fn transcript(
        &self,
        frame: AgentChatTranscriptFrame,
    ) -> Result<AgentChatTranscriptFrame, String> {
        self.agent_chat_transcript(frame).map_err(String::from)
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

#[cfg(test)]
#[path = "agent_chat_subscription_tests.rs"]
mod tests;
