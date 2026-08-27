//! Durable, revisioned per-conversation CLI-launch configuration.

use serde::{Deserialize, Serialize};

use crate::AgentChatConversationId;

/// One immutable revision of a conversation's advanced launch configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatConversationConfigRecord {
    pub conversation_id: AgentChatConversationId,
    pub revision: u64,
    /// Free-text system-prompt content. Appended or substituted per `append_system_prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// When true, `system_prompt` appends to the provider's default prompt; when false, it
    /// replaces it. Ignored when `system_prompt` is absent.
    #[serde(default)]
    pub append_system_prompt: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// Canonically sorted, unique tool names this conversation's provider must never invoke.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
}

/// A configured field the conversation's current provider selection cannot honor.
///
/// Configuration is still accepted and persisted for every provider, since a conversation may
/// later switch providers; this only reports what the *current* selection cannot apply.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentChatConversationConfigUnsupportedField {
    SystemPromptOverride,
    MaxTurns,
    DisallowedTools,
}

#[cfg(test)]
mod tests {
    use super::{AgentChatConversationConfigRecord, AgentChatConversationConfigUnsupportedField};
    use crate::AgentChatConversationId;
    use serde_json::json;

    #[test]
    fn record_omits_absent_optional_fields_and_defaults_the_rest() {
        let record = AgentChatConversationConfigRecord {
            conversation_id: AgentChatConversationId("conversation-1".into()),
            revision: 1,
            system_prompt: None,
            append_system_prompt: false,
            max_turns: None,
            disallowed_tools: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&record).unwrap(),
            json!({
                "conversationId": "conversation-1", "revision": 1,
                "appendSystemPrompt": false, "disallowedTools": []
            })
        );
    }

    #[test]
    fn older_record_json_defaults_missing_fields() {
        let record: AgentChatConversationConfigRecord = serde_json::from_value(json!({
            "conversationId": "conversation-1", "revision": 1
        }))
        .unwrap();
        assert_eq!(record.system_prompt, None);
        assert!(!record.append_system_prompt);
        assert_eq!(record.max_turns, None);
        assert!(record.disallowed_tools.is_empty());
    }

    #[test]
    fn unsupported_field_names_are_stable_and_camel_case() {
        assert_eq!(
            serde_json::to_value(AgentChatConversationConfigUnsupportedField::MaxTurns).unwrap(),
            json!("maxTurns")
        );
    }
}
