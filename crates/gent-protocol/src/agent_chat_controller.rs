//! Typed, resumable selected-conversation controller stream frames.

use gent_types::{
    AgentChatConversationDetail, ConversationStatus, HostEpoch, NormalizedTranscriptEvent,
    NormalizedTranscriptPage,
};
use serde::{Deserialize, Serialize};

/// Negotiates the long-lived selected-conversation controller stream.
///
/// The shipped observer daemon deliberately does not advertise this capability.
pub const AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY: &str = "agent-chat-controller-stream-v1";

/// A complete selected-conversation projection that replaces client state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatControllerSnapshot {
    /// Daemon generation which owns every value in this projection.
    pub host_epoch: HostEpoch,
    /// The selected durable conversation without provider-native session data.
    pub conversation: AgentChatConversationDetail,
    /// A normalized durable transcript page for the selected conversation.
    pub transcript: NormalizedTranscriptPage,
    /// Last transcript cursor applied to this projection.
    pub cursor: u64,
    /// Optional durable hierarchy and lifecycle projection.
    pub status: Option<ConversationStatus>,
}

/// An additive change to a selected-conversation projection.
///
/// New variants may be added without making raw provider payloads part of this contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatControllerDelta {
    /// One daemon-normalized transcript event for the attached conversation.
    Transcript {
        host_epoch: HostEpoch,
        event: NormalizedTranscriptEvent,
    },
}

/// A typed reason for ending a controller stream.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatControllerStreamEnd {
    /// The daemon is closing its current host epoch.
    ServerClosing,
    /// The client must attach again and replace its local projection.
    ResyncRequired,
    /// The selected conversation can no longer be read by this stream.
    ConversationUnavailable,
}

/// Frames used only after a negotiated [`AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY`] handshake.
///
/// `Snapshot` and `Resync` replace client state. `Delta` is intentionally limited
/// to normalized transcript changes today. `Ack` records client progress only; it
/// never alters daemon retention or the durable transcript.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatControllerStreamFrame {
    /// Attaches to one selected conversation after an already rendered transcript cursor.
    Attach {
        conversation_id: String,
        after_cursor: u64,
    },
    /// Initial complete projection after attach.
    Snapshot(AgentChatControllerSnapshot),
    /// One ordered additive projection change.
    Delta(AgentChatControllerDelta),
    /// Client progress after applying a durable transcript cursor.
    Ack { cursor: u64 },
    /// Terminal stream state without a raw provider error payload.
    End {
        reason: AgentChatControllerStreamEnd,
    },
    /// Replaces stale client state after an epoch or cursor discontinuity.
    Resync(AgentChatControllerSnapshot),
}

#[cfg(test)]
mod tests {
    use super::{
        AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY, AgentChatControllerDelta,
        AgentChatControllerSnapshot, AgentChatControllerStreamEnd, AgentChatControllerStreamFrame,
    };
    use gent_types::{
        AgentChatConversationDetail, AgentChatConversationSummary, AgentChatEffort, AgentChatMode,
        AgentChatProvider, AgentChatSelection, ConversationStatus, HostEpoch,
        NormalizedTranscriptEvent, NormalizedTranscriptKind, NormalizedTranscriptPage,
    };
    use serde_json::json;

    #[test]
    fn controller_stream_has_stable_frames_and_capability() {
        let snapshot = snapshot();
        assert_eq!(
            serde_json::to_value(AgentChatControllerStreamFrame::Attach {
                conversation_id: "conversation-1".into(),
                after_cursor: 4,
            })
            .unwrap(),
            json!({ "type": "attach", "body": { "conversationId": "conversation-1", "afterCursor": 4 } })
        );
        assert_eq!(
            serde_json::to_value(AgentChatControllerStreamFrame::Delta(
                AgentChatControllerDelta::Transcript {
                    host_epoch: HostEpoch(7),
                    event: event(5)
                }
            ))
            .unwrap()["type"],
            "delta"
        );
        assert_eq!(
            serde_json::from_value::<AgentChatControllerStreamFrame>(
                serde_json::to_value(AgentChatControllerStreamFrame::Resync(snapshot)).unwrap()
            )
            .unwrap()
            .as_resync_cursor(),
            Some(4)
        );
        assert_eq!(
            AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY,
            "agent-chat-controller-stream-v1"
        );
    }

    #[test]
    fn controller_stream_rejects_unknown_fields_at_every_boundary() {
        let attach = json!({
            "type": "attach",
            "body": { "conversationId": "conversation-1", "afterCursor": 0, "providerSessionId": "private" }
        });
        assert!(serde_json::from_value::<AgentChatControllerStreamFrame>(attach).is_err());
        let snapshot = json!({
            "hostEpoch": 7,
            "conversation": conversation(),
            "transcript": page(),
            "cursor": 4,
            "status": null,
            "providerPayload": {}
        });
        assert!(serde_json::from_value::<AgentChatControllerSnapshot>(snapshot).is_err());
        let end =
            json!({ "type": "end", "body": { "reason": "serverClosing", "detail": "private" } });
        assert!(serde_json::from_value::<AgentChatControllerStreamFrame>(end).is_err());
    }

    trait ResyncCursor {
        fn as_resync_cursor(&self) -> Option<u64>;
    }

    impl ResyncCursor for AgentChatControllerStreamFrame {
        fn as_resync_cursor(&self) -> Option<u64> {
            match self {
                Self::Resync(snapshot) => Some(snapshot.cursor),
                _ => None,
            }
        }
    }

    fn snapshot() -> AgentChatControllerSnapshot {
        AgentChatControllerSnapshot {
            host_epoch: HostEpoch(7),
            conversation: conversation(),
            transcript: page(),
            cursor: 4,
            status: Some(ConversationStatus {
                conversation_id: "conversation-1".into(),
                runs: Vec::new(),
            }),
        }
    }

    fn conversation() -> AgentChatConversationDetail {
        AgentChatConversationDetail {
            summary: AgentChatConversationSummary {
                conversation_id: "conversation-1".into(),
                title: Some("Chat".into()),
                updated_at_unix_ms: 1,
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Ask,
                },
            },
            runs: Vec::new(),
        }
    }

    fn page() -> NormalizedTranscriptPage {
        NormalizedTranscriptPage {
            conversation_id: "conversation-1".into(),
            events: vec![event(4)],
            next_after_cursor: Some(4),
        }
    }

    fn event(cursor: u64) -> NormalizedTranscriptEvent {
        NormalizedTranscriptEvent {
            cursor,
            event_id: format!("event-{cursor}"),
            turn_id: "turn-1".into(),
            run_id: "run-1".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "normalized".into(),
            is_partial: false,
        }
    }

    #[test]
    fn end_reason_stays_closed() {
        assert!(serde_json::from_str::<AgentChatControllerStreamEnd>("\"later\"").is_err());
    }
}
