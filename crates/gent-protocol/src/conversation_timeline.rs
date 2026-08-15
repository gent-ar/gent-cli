//! Additive, dedicated frames for a non-content conversation timeline read.

use gent_types::ConversationTimeline;
use serde::{Deserialize, Serialize};

/// Capability required before the conversation-timeline endpoint accepts a request.
pub const CONVERSATION_TIMELINE_CAPABILITY: &str = "conversation-timeline-v1";

/// Frames carried only by the dedicated conversation-timeline local endpoint.
///
/// This is separate from [`crate::WireFrame`] so the established command and event contract is
/// unchanged. The response excludes transcript content and provider-native session identifiers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum ConversationTimelineFrame {
    TimelineRequest { conversation_id: String },
    Timeline(ConversationTimeline),
}

#[cfg(test)]
mod tests {
    use super::{CONVERSATION_TIMELINE_CAPABILITY, ConversationTimelineFrame};

    #[test]
    fn request_has_a_distinct_wire_discriminant() {
        let frame = ConversationTimelineFrame::TimelineRequest {
            conversation_id: "conversation-1".into(),
        };
        let value = serde_json::to_value(frame).unwrap();
        assert_eq!(value["type"], "timelineRequest");
        assert_eq!(CONVERSATION_TIMELINE_CAPABILITY, "conversation-timeline-v1");
    }
}
