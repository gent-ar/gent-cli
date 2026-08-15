//! Additive, dedicated frames for the read-only conversation-status endpoint.

use gent_types::ConversationStatus;
use serde::{Deserialize, Serialize};

/// Capability required before the conversation-status endpoint accepts a request.
pub const CONVERSATION_STATUS_CAPABILITY: &str = "conversation-status-v1";

/// Frames carried only by the dedicated conversation-status local endpoint.
///
/// This intentionally remains separate from [`crate::WireFrame`]. Existing command, receipt,
/// and global event semantics therefore remain stable as this read-only extension evolves.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum ConversationStatusFrame {
    Request { conversation_id: String },
    Status(ConversationStatus),
}
