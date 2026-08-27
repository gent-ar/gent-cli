//! Capability-gated reads of daemon-normalized agent-chat state.

use std::{collections::HashSet, path::PathBuf};

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
    AgentChatConversationFrame, AgentChatTranscriptFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationSummary, NormalizedTranscriptPage,
};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn summary(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
) -> Result<AgentChatConversationSummary, Box<dyn std::error::Error>> {
    let reply = conversation(
        data_dir,
        no_autostart,
        AgentChatConversationFrame::SummaryRequest {
            conversation_id: conversation_id.clone(),
        },
    )
    .await?;
    match reply {
        AgentChatConversationFrame::Summary(value) if value.conversation_id == conversation_id => {
            Ok(value)
        }
        _ => Err("daemon did not return the requested agent-chat summary".into()),
    }
}

pub(crate) async fn detail(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
) -> Result<AgentChatConversationDetail, Box<dyn std::error::Error>> {
    let reply = conversation(
        data_dir,
        no_autostart,
        AgentChatConversationFrame::DetailRequest {
            conversation_id: conversation_id.clone(),
        },
    )
    .await?;
    match reply {
        AgentChatConversationFrame::Detail(value)
            if value.summary.conversation_id == conversation_id =>
        {
            Ok(value)
        }
        _ => Err("daemon did not return the requested agent-chat detail".into()),
    }
}

pub(crate) async fn transcript(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    after_cursor: Option<u64>,
    limit: u16,
) -> Result<NormalizedTranscriptPage, Box<dyn std::error::Error>> {
    if limit == 0 {
        return Err("transcript limit must be at least 1".into());
    }
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require(&capabilities.0, AGENT_CHAT_TRANSCRIPT_CAPABILITY)?;
    write_json_frame(
        &mut stream,
        &AgentChatTranscriptFrame::PageRequest {
            conversation_id: conversation_id.clone(),
            after_cursor,
            limit,
        },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    let reply = decode::<AgentChatTranscriptFrame>(raw, "agent-chat transcript")?;
    match reply {
        AgentChatTranscriptFrame::Page(page)
            if valid_page(&page, &conversation_id, after_cursor) =>
        {
            Ok(page)
        }
        _ => Err("daemon returned an invalid agent-chat transcript page".into()),
    }
}

pub(crate) async fn transcript_all(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
) -> Result<NormalizedTranscriptPage, Box<dyn std::error::Error>> {
    let mut after_cursor = None;
    let mut events = Vec::new();
    let mut event_ids = HashSet::new();
    loop {
        let page = transcript(
            data_dir.clone(),
            no_autostart,
            conversation_id.clone(),
            after_cursor,
            100,
        )
        .await?;
        after_cursor = page.next_after_cursor;
        for event in page.events {
            if !event_ids.insert(event.event_id.clone()) {
                return Err("daemon returned a duplicate transcript event identity".into());
            }
            events.push(event);
        }
        if after_cursor.is_none() {
            return Ok(NormalizedTranscriptPage {
                conversation_id,
                events,
                next_after_cursor: None,
            });
        }
    }
}

async fn conversation(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: AgentChatConversationFrame,
) -> Result<AgentChatConversationFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require(&capabilities.0, AGENT_CHAT_CONVERSATIONS_CAPABILITY)?;
    write_json_frame(&mut stream, &request).await?;
    decode(
        read_json_frame(&mut stream).await?,
        "agent-chat conversation",
    )
}

fn require(capabilities: &[String], capability: &str) -> Result<(), Box<dyn std::error::Error>> {
    capabilities
        .iter()
        .any(|item| item == capability)
        .then_some(())
        .ok_or_else(|| "daemon does not support agent-chat reads; upgrade gentd".into())
}

fn decode<T: serde::de::DeserializeOwned>(
    raw: Value,
    label: &str,
) -> Result<T, Box<dyn std::error::Error>> {
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    serde_json::from_value(raw).map_err(|_| format!("daemon did not return {label}").into())
}

fn valid_page(
    page: &NormalizedTranscriptPage,
    conversation_id: &str,
    after_cursor: Option<u64>,
) -> bool {
    if page.conversation_id != conversation_id {
        return false;
    }
    let mut cursor = after_cursor.unwrap_or_default();
    let mut event_ids = HashSet::new();
    for event in &page.events {
        if event.cursor <= cursor
            || event.event_id.is_empty()
            || event.turn_id.is_empty()
            || event.run_id.is_empty()
            || !event_ids.insert(&event.event_id)
        {
            return false;
        }
        cursor = event.cursor;
    }
    page.next_after_cursor.is_none_or(|next| {
        !page.events.is_empty() && next == cursor && next > after_cursor.unwrap_or_default()
    })
}

#[cfg(all(test, unix))]
#[path = "reads_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "reads_pagination_tests.rs"]
mod pagination_tests;
