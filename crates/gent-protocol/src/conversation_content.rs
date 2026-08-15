//! Negotiated, local-only reads of durable user prompt content.

use gent_types::{ConversationContentCursor, ConversationContentPage};
use serde::{Deserialize, Serialize};

pub const CONVERSATION_CONTENT_CAPABILITY: &str = "conversation-content-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum ConversationContentFrame {
    Request {
        conversation_id: String,
        before: Option<ConversationContentCursor>,
        limit: u16,
    },
    Page(ConversationContentPage),
}
