//! Additive frames for content-free local conversation discovery.

use gent_types::ConversationListItem;
use serde::{Deserialize, Serialize};

/// Negotiated capability for a content-free conversation index.
pub const CONVERSATION_INDEX_CAPABILITY: &str = "conversation-index-v1";

/// Request and response frames for selecting a durable conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum ConversationIndexFrame {
    Request,
    Index(Vec<ConversationListItem>),
}
