//! Protocol-only client for private, paginated local user-prompt content.

use std::path::PathBuf;

use gent_protocol::{
    CONVERSATION_CONTENT_CAPABILITY, ConversationContentFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{ConversationContentCursor, ConversationContentPage};
use serde_json::Value;

use crate::local_ipc::connect_and_negotiate;

pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    before: Option<ConversationContentCursor>,
    limit: u16,
) -> Result<ConversationContentPage, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|item| item == CONVERSATION_CONTENT_CAPABILITY)
    {
        return Err("daemon does not support private conversation content; upgrade gentd".into());
    }
    write_json_frame(
        &mut stream,
        &ConversationContentFrame::Request {
            conversation_id,
            before,
            limit,
        },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(ConversationContentFrame::Page(page)) = serde_json::from_value(raw.clone()) {
        return Ok(page);
    }
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw) {
        return Err(message.into());
    }
    Err("daemon did not return conversation content".into())
}
