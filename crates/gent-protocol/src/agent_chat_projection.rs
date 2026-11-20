use gent_types::{
    AgentChatConversationDetail, AgentChatConversationSummary, ConversationActivityFact,
    ConversationTurnCursor, GoalRecord, NormalizedSessionLifecycle, NormalizedTranscriptEvent,
    TurnRecord,
};
use serde::{Deserialize, Serialize};

pub const AGENT_CHAT_PROJECTION_CAPABILITY: &str = "agent-chat-projection-v1";
pub const AGENT_CHAT_PROJECTION_WINDOW_CAPABILITY: &str = "agent-chat-projection-window-v1";
pub const COMMAND_CATALOG_ID: &str = "commands";

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
    pub unreadable: Vec<UnreadableProjectionRecord>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectionRecordKind {
    Transcript,
    Activity,
    Lifecycle,
    Summary,
    Unrecognized,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnreadableProjectionRecord {
    pub cursor: ProjectionCursor,
    pub record_kind: ProjectionRecordKind,
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
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatProjectionDelta {
    Transcript {
        cursor: ProjectionCursor,
        event: Box<NormalizedTranscriptEvent>,
    },
    Activity {
        cursor: ProjectionCursor,
        fact: Box<ConversationActivityFact>,
    },
    Lifecycle {
        cursor: ProjectionCursor,
        lifecycle: NormalizedSessionLifecycle,
    },
    Catalog {
        cursor: ProjectionCursor,
        catalog: ProjectionCatalog,
    },
    Summary {
        cursor: ProjectionCursor,
        run_id: String,
        summary: Box<AgentChatConversationSummary>,
    },
    UnreadableRecord {
        cursor: ProjectionCursor,
        record_kind: ProjectionRecordKind,
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
        snapshot: Box<AgentChatProjectionSnapshot>,
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
    ConversationTurnCursorsRequest {
        request_id: String,
        conversation_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_cursor: Option<ProjectionCursor>,
        limit: u16,
    },
    ConversationTurnCursors {
        request_id: String,
        conversation_id: String,
        turns: Vec<ConversationTurnCursor>,
        next_before_cursor: Option<ProjectionCursor>,
    },
    ConversationWindowRequest {
        request_id: String,
        conversation_id: String,
        from_cursor: ProjectionCursor,
    },
    ConversationWindow {
        request_id: String,
        conversation_id: String,
        from_cursor: ProjectionCursor,
        last_turn: Option<TurnRecord>,
        carried: Vec<AgentChatProjectionDelta>,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        AGENT_CHAT_PROJECTION_CAPABILITY, AgentChatProjectionDelta, AgentChatProjectionFrame,
        ProjectionCursor, ProjectionRecordKind,
    };
    use gent_types::{NormalizedTranscriptEvent, NormalizedTranscriptKind};
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
    fn transcript_delta_frames_keep_their_unboxed_wire_shape() {
        let frame = AgentChatProjectionFrame::Delta {
            request_id: "request-1".into(),
            delta: AgentChatProjectionDelta::Transcript {
                cursor: ProjectionCursor { value: 240 },
                event: Box::new(NormalizedTranscriptEvent {
                    cursor: 240,
                    event_id: "event-240".into(),
                    turn_id: "turn-9".into(),
                    run_id: "run-1".into(),
                    kind: NormalizedTranscriptKind::AssistantMessage,
                    text: "Latest reply".into(),
                    is_partial: false,
                    failure_code: None,
                    resets_at: None,
                    origin: None,
                    attachments: Vec::new(),
                }),
            },
        };
        assert_eq!(
            serde_json::to_string(&frame).expect("frame serializes"),
            concat!(
                r#"{"type":"delta","body":{"requestId":"request-1","delta":{"type":"transcript","#,
                r#""body":{"cursor":{"value":240},"event":{"cursor":240,"eventId":"event-240","#,
                r#""turnId":"turn-9","runId":"run-1","kind":"assistantMessage","#,
                r#""text":"Latest reply","isPartial":false}}}}}"#
            )
        );
    }

    #[test]
    fn an_unreadable_record_is_one_typed_delta_at_its_cursor() {
        let frame = AgentChatProjectionFrame::Delta {
            request_id: "request-1".into(),
            delta: AgentChatProjectionDelta::UnreadableRecord {
                cursor: ProjectionCursor { value: 11004 },
                record_kind: ProjectionRecordKind::Lifecycle,
            },
        };
        assert_eq!(
            serde_json::to_value(frame).expect("frame serializes"),
            json!({
                "type": "delta",
                "body": {
                    "requestId": "request-1",
                    "delta": {
                        "type": "unreadableRecord",
                        "body": { "cursor": { "value": 11004 }, "recordKind": "lifecycle" }
                    }
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
