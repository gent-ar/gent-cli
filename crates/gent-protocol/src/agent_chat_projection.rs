use gent_types::{
    AgentChatConversationDetail, ConversationActivityFact, GoalRecord, NormalizedSessionLifecycle,
    NormalizedTranscriptEvent,
};
use serde::{Deserialize, Serialize};

pub const AGENT_CHAT_PROJECTION_CAPABILITY: &str = "agent-chat-projection-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisplayRecord {
    pub id: String,
    pub label: String,
    pub ordering: u32,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub explanation: Option<String>,
    pub requires_confirmation: bool,
    pub scope: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionCursor {
    pub value: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatProjectionSnapshot {
    pub conversation: AgentChatConversationDetail,
    pub cursor: ProjectionCursor,
    pub catalogs: Vec<ProjectionCatalog>,
    pub transcript: Vec<NormalizedTranscriptEvent>,
    pub activity: Vec<ConversationActivityFact>,
    pub goal: Option<GoalRecord>,
    pub transcript_truncated: bool,
    pub activity_truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionCatalog {
    pub catalog_id: String,
    pub selected_id: Option<String>,
    pub records: Vec<DisplayRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatProjectionDelta {
    Transcript {
        cursor: ProjectionCursor,
        event: NormalizedTranscriptEvent,
    },
    Activity {
        cursor: ProjectionCursor,
        fact: ConversationActivityFact,
    },
    Lifecycle {
        cursor: ProjectionCursor,
        lifecycle: NormalizedSessionLifecycle,
    },
    Catalog {
        cursor: ProjectionCursor,
        catalog: ProjectionCatalog,
    },
    ResyncRequired {
        cursor: ProjectionCursor,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatProjectionFrame {
    ConversationSnapshotRequest {
        request_id: String,
        conversation_id: String,
        transcript_limit: u16,
        activity_limit: u16,
    },
    ConversationSnapshot {
        request_id: String,
        snapshot: AgentChatProjectionSnapshot,
    },
    FollowConversation {
        request_id: String,
        conversation_id: String,
        after_cursor: ProjectionCursor,
    },
    Delta {
        request_id: String,
        delta: AgentChatProjectionDelta,
    },
    Ended {
        request_id: String,
        cursor: ProjectionCursor,
    },
}

#[cfg(test)]
mod tests {
    use super::{AGENT_CHAT_PROJECTION_CAPABILITY, AgentChatProjectionFrame, ProjectionCursor};
    use serde_json::json;

    #[test]
    fn projection_frames_preserve_request_and_cursor_identity() {
        let frame = AgentChatProjectionFrame::FollowConversation {
            request_id: "request-1".into(),
            conversation_id: "conversation-1".into(),
            after_cursor: ProjectionCursor { value: 7 },
        };
        assert_eq!(AGENT_CHAT_PROJECTION_CAPABILITY, "agent-chat-projection-v1");
        assert_eq!(
            serde_json::to_value(frame).expect("frame serializes"),
            json!({
                "type": "followConversation",
                "body": {
                    "requestId": "request-1",
                    "conversationId": "conversation-1",
                    "afterCursor": { "value": 7 }
                }
            })
        );
    }

    #[test]
    fn projection_frames_reject_unknown_fields() {
        let frame = json!({
            "type": "conversationSnapshotRequest",
            "body": {
                "requestId": "request-1",
                "conversationId": "conversation-1",
                "transcriptLimit": 20,
                "activityLimit": 20,
                "provider": "claude"
            }
        });
        assert!(serde_json::from_value::<AgentChatProjectionFrame>(frame).is_err());
    }
}
