//! Capability-gated, read-only frames for a future agent-chat client.

use gent_types::{
    AgentChatConversationDetail, AgentChatConversationSummary, NormalizedTranscriptPage,
};
use serde::{Deserialize, Serialize};

/// Required before a client may request agent-chat conversation metadata.
pub const AGENT_CHAT_CONVERSATIONS_CAPABILITY: &str = "agent-chat-conversations-v1";

/// Required before a client may request normalized transcript content.
pub const AGENT_CHAT_TRANSCRIPT_CAPABILITY: &str = "agent-chat-transcript-v1";

/// Read-only frames for conversation selection and detail.
///
/// This contract does not grant prompt submission, provider spawning, or settings mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatConversationFrame {
    SummaryRequest { conversation_id: String },
    Summary(AgentChatConversationSummary),
    DetailRequest { conversation_id: String },
    Detail(AgentChatConversationDetail),
}

/// Read-only frames for cursor-paginated normalized transcript content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatTranscriptFrame {
    PageRequest {
        conversation_id: String,
        after_cursor: Option<u64>,
        limit: u16,
    },
    Page(NormalizedTranscriptPage),
}

#[cfg(test)]
mod tests {
    use super::{
        AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
        AgentChatConversationFrame, AgentChatTranscriptFrame,
    };
    use serde_json::json;

    #[test]
    fn chat_frames_have_stable_discriminants_and_capabilities() {
        assert_eq!(
            AGENT_CHAT_CONVERSATIONS_CAPABILITY,
            "agent-chat-conversations-v1"
        );
        assert_eq!(AGENT_CHAT_TRANSCRIPT_CAPABILITY, "agent-chat-transcript-v1");
        assert_eq!(
            serde_json::to_value(AgentChatConversationFrame::DetailRequest {
                conversation_id: "conversation-1".into(),
            })
            .unwrap(),
            json!({ "type": "detailRequest", "body": { "conversationId": "conversation-1" } })
        );
    }

    #[test]
    fn frame_contract_rejects_unknown_request_fields() {
        let frame = json!({
            "type": "pageRequest",
            "body": { "conversationId": "c1", "afterCursor": null, "limit": 20, "sessionId": "private" }
        });
        assert!(serde_json::from_value::<AgentChatTranscriptFrame>(frame).is_err());
    }
}
