//! Negotiated local IPC for durable per-conversation advanced launch configuration.

use gent_types::{AgentChatConversationConfigRecord, AgentChatConversationConfigUnsupportedField};
use serde::{Deserialize, Serialize};

/// Negotiated capability for local conversation-config reads and explicit revisions.
pub const AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY: &str = "agent-chat-conversation-config-v1";

/// One finite conversation-config exchange. Provider execution is never part of this protocol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatConversationConfigFrame {
    Current {
        request_id: String,
        conversation_id: String,
    },
    Save {
        request_id: String,
        config: AgentChatConversationConfigRecord,
    },
    CurrentConfig {
        request_id: String,
        config: Option<AgentChatConversationConfigRecord>,
        unsupported_for_provider: Vec<AgentChatConversationConfigUnsupportedField>,
    },
    Saved {
        request_id: String,
        config: AgentChatConversationConfigRecord,
        unsupported_for_provider: Vec<AgentChatConversationConfigUnsupportedField>,
    },
}

#[cfg(test)]
mod tests {
    use super::{AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY, AgentChatConversationConfigFrame};
    use gent_types::{AgentChatConversationConfigRecord, AgentChatConversationId};
    use serde_json::json;

    fn config() -> AgentChatConversationConfigRecord {
        AgentChatConversationConfigRecord {
            conversation_id: AgentChatConversationId("conversation-1".into()),
            revision: 1,
            system_prompt: Some("Be concise.".into()),
            append_system_prompt: true,
            max_turns: Some(10),
            disallowed_tools: vec!["shell:rm".into()],
        }
    }

    #[test]
    fn current_frame_round_trips_over_a_plain_conversation_identity() {
        let frame = AgentChatConversationConfigFrame::Current {
            request_id: "request-1".into(),
            conversation_id: "conversation-1".into(),
        };
        assert_eq!(
            serde_json::to_value(&frame).unwrap(),
            json!({
                "type": "current",
                "body": { "requestId": "request-1", "conversationId": "conversation-1" }
            })
        );
        assert_eq!(
            AGENT_CHAT_CONVERSATION_CONFIG_CAPABILITY,
            "agent-chat-conversation-config-v1"
        );
    }

    #[test]
    fn saved_response_carries_the_unsupported_field_list() {
        let frame = AgentChatConversationConfigFrame::Saved {
            request_id: "request-1".into(),
            config: config(),
            unsupported_for_provider: vec![
                gent_types::AgentChatConversationConfigUnsupportedField::MaxTurns,
            ],
        };
        let value = serde_json::to_value(&frame).unwrap();
        assert_eq!(value["body"]["unsupportedForProvider"], json!(["maxTurns"]));
    }

    #[test]
    fn frame_rejects_unknown_fields() {
        let frame = json!({
            "type": "save",
            "body": {
                "requestId": "request-1", "config": config(), "providerSessionId": "never-public"
            }
        });
        assert!(serde_json::from_value::<AgentChatConversationConfigFrame>(frame).is_err());
    }
}
