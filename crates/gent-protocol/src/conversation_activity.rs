//! Additive frames for cursor-resumable, content-free conversation activity facts.

use gent_types::ConversationActivityPage;
use serde::{Deserialize, Serialize};

/// Capability required before an authoritative activity endpoint may accept requests.
///
/// The shipped observer daemon deliberately does not advertise this capability.
pub const CONVERSATION_ACTIVITY_CAPABILITY: &str = "conversation-activity-v1";

/// Dedicated activity-history frames, separate from commands and global event streaming.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ConversationActivityFrame {
    /// Starts a read at a durable activity cursor for one immutable conversation run.
    Request {
        conversation_id: String,
        run_id: String,
        after_cursor: u64,
    },
    /// One bounded page of immutable facts strictly after the requested cursor.
    Facts(ConversationActivityPage),
}

#[cfg(test)]
mod tests {
    use super::{CONVERSATION_ACTIVITY_CAPABILITY, ConversationActivityFrame};

    #[test]
    fn request_is_a_distinct_cursor_bound_contract() {
        let frame = ConversationActivityFrame::Request {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            after_cursor: 9,
        };
        let value = serde_json::to_value(frame).unwrap();
        assert_eq!(value["type"], "request");
        assert_eq!(value["body"]["afterCursor"], 9);
        assert_eq!(CONVERSATION_ACTIVITY_CAPABILITY, "conversation-activity-v1");
    }
}
