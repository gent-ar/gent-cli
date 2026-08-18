//! Typed, cursor-resumable following of one durable turn.

use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, NormalizedTranscriptEvent,
    TurnTerminal,
};
use serde::{Deserialize, Serialize};

/// Required before a client may follow one normalized durable turn to settlement.
///
/// This remains absent from observer and persistence-only capability catalogs.
pub const AGENT_CHAT_TURN_FOLLOW_CAPABILITY: &str = "agent-chat-turn-follow-v1";

/// Frames for a single turn-scoped transcript subscription.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatTurnFollowFrame {
    Follow {
        request_id: AgentChatRequestId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        turn_id: String,
        after_cursor: u64,
    },
    Event {
        request_id: AgentChatRequestId,
        event: NormalizedTranscriptEvent,
    },
    Terminal {
        request_id: AgentChatRequestId,
        terminal: TurnTerminal,
    },
    Ended {
        request_id: AgentChatRequestId,
        reason: AgentChatTurnFollowEnd,
    },
}

/// Why a turn follower must stop without receiving a terminal record.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatTurnFollowEnd {
    ResyncRequired,
    ServerClosing,
}

#[cfg(test)]
mod tests {
    use super::{AGENT_CHAT_TURN_FOLLOW_CAPABILITY, AgentChatTurnFollowFrame};
    use serde_json::json;

    #[test]
    fn follow_is_correlated_and_closed_to_private_fields() {
        assert_eq!(
            AGENT_CHAT_TURN_FOLLOW_CAPABILITY,
            "agent-chat-turn-follow-v1"
        );
        let frame = json!({ "type": "follow", "body": {
            "requestId": "request-1", "conversationId": "conversation-1", "runId": "run-1",
            "turnId": "turn-1", "afterCursor": 3, "providerSessionId": "private"
        }});
        assert!(serde_json::from_value::<AgentChatTurnFollowFrame>(frame).is_err());
    }
}
