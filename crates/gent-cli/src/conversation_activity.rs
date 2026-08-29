//! Protocol-only reader for authority-gated, content-free conversation activity facts.

use std::path::PathBuf;

use gent_protocol::{
    CONVERSATION_ACTIVITY_CAPABILITY, ConversationActivityFrame, Hello, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{ConversationActivityPage, PROTOCOL_MAX, PROTOCOL_MIN};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::local_ipc::{client_capabilities, connect_or_start, default_data_dir};

/// One verified page of content-free activity facts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ActivityRead(pub ConversationActivityPage);

/// Reads one cursor-ordered activity page without creating a receipt.
///
/// # Errors
/// Returns an error when the daemon is unavailable, unnegotiated, or not authoritative.
pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    run_id: String,
    after_cursor: u64,
) -> Result<ActivityRead, Box<dyn std::error::Error>> {
    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    let mut stream = connect_or_start(&data_dir, no_autostart).await?;
    write_frame(
        &mut stream,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: client_capabilities(),
        }),
    )
    .await?;
    let WireFrame::Negotiated(negotiated) = read_frame(&mut stream).await? else {
        return Err("daemon did not negotiate protocol".into());
    };
    if !negotiated
        .capabilities
        .0
        .iter()
        .any(|capability| capability == CONVERSATION_ACTIVITY_CAPABILITY)
    {
        return Err("daemon does not support authoritative conversation activity".into());
    }
    write_json_frame(
        &mut stream,
        &ConversationActivityFrame::Request {
            conversation_id: conversation_id.clone(),
            run_id: run_id.clone(),
            after_cursor,
        },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(frame) = serde_json::from_value(raw.clone()) {
        return decode(frame, &conversation_id, &run_id, after_cursor).map_err(Into::into);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return conversation activity facts".into())
}

pub(crate) async fn all(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    run_id: String,
) -> Result<Vec<gent_types::ConversationActivityFact>, Box<dyn std::error::Error>> {
    let mut after_cursor = 0;
    let mut facts = Vec::new();
    loop {
        let page = request(
            data_dir.clone(),
            no_autostart,
            conversation_id.clone(),
            run_id.clone(),
            after_cursor,
        )
        .await?
        .0;
        if let Some(next) = page.next_after_cursor {
            after_cursor = next;
        } else {
            facts.extend(page.facts);
            return Ok(facts);
        }
        facts.extend(page.facts);
    }
}

fn decode(
    frame: ConversationActivityFrame,
    conversation_id: &str,
    run_id: &str,
    after_cursor: u64,
) -> Result<ActivityRead, String> {
    let ConversationActivityFrame::Facts(page) = frame else {
        return Err("daemon returned an activity request instead of facts".into());
    };
    validate_page(&page, conversation_id, run_id, after_cursor)?;
    Ok(ActivityRead(page))
}

fn validate_page(
    page: &ConversationActivityPage,
    conversation_id: &str,
    run_id: &str,
    after_cursor: u64,
) -> Result<(), String> {
    let mut cursor = after_cursor;
    for fact in &page.facts {
        let scope = fact.scope();
        if scope.conversation_id != conversation_id || scope.run_id != run_id {
            return Err("daemon returned activity for another conversation run".into());
        }
        if scope.cursor <= cursor {
            return Err("daemon returned non-monotonic activity cursors".into());
        }
        cursor = scope.cursor;
    }
    if page
        .next_after_cursor
        .is_some_and(|next| page.facts.is_empty() || next != cursor)
    {
        return Err("daemon returned an inconsistent activity page cursor".into());
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "conversation_activity_tests.rs"]
mod tests;
