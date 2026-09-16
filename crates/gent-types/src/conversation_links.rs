use serde::{Deserialize, Serialize};

use crate::DurableTurnPhase;

pub const MAX_CONVERSATION_LABEL_BYTES: usize = 120;
pub const MAX_CONVERSATION_MESSAGE_PREVIEW_BYTES: usize = 200;
pub const MAX_CONVERSATION_WAIT_REPLY_BYTES: usize = 2000;
pub const MAX_CONVERSATION_WAIT_SECONDS: u32 = 900;
pub const MAX_CONVERSATION_WAIT_TARGETS: usize = 16;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationLink {
    pub parent_conversation_id: String,
    pub child_conversation_id: String,
    pub label: String,
    pub created_run_id: String,
    pub created_turn_id: String,
    pub created_at_unix_seconds: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationMessageDelivery {
    Queued,
    Steered,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationWaitTarget {
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<DurableTurnPhase>,
}

impl ConversationWaitTarget {
    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.phase.is_none_or(DurableTurnPhase::is_terminal)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationWaitResult {
    #[serde(flatten)]
    pub target: ConversationWaitTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationThreadState {
    pub conversation_id: String,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<DurableTurnPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_assistant_text: Option<String>,
    pub last_activity_unix_seconds: u64,
}

impl ConversationThreadState {
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.phase.is_some_and(|phase| !phase.is_terminal())
    }

    #[must_use]
    pub fn fact_turn_id(&self) -> String {
        self.turn_id
            .clone()
            .unwrap_or_else(|| format!("links:{}", self.conversation_id))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedConversationSummary {
    pub conversation_id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<DurableTurnPhase>,
    pub last_activity_unix_seconds: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedConversations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<LinkedConversationSummary>,
    pub children: Vec<LinkedConversationSummary>,
}

#[must_use]
pub fn bounded_excerpt(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[must_use]
pub fn valid_conversation_label(label: &str) -> bool {
    !label.trim().is_empty() && label.len() <= MAX_CONVERSATION_LABEL_BYTES
}

#[cfg(test)]
mod tests {
    use super::{
        ConversationWaitResult, ConversationWaitTarget, bounded_excerpt, valid_conversation_label,
    };
    use crate::DurableTurnPhase;

    #[test]
    fn a_target_that_never_took_a_turn_omits_its_phase_and_is_settled() {
        let target = ConversationWaitTarget {
            conversation_id: "conversation-1".into(),
            label: String::new(),
            phase: None,
        };
        assert!(target.is_settled());
        assert_eq!(
            serde_json::to_value(&target).unwrap(),
            serde_json::json!({ "conversationId": "conversation-1" })
        );
    }

    #[test]
    fn a_settled_result_flattens_its_target_beside_a_bounded_reply() {
        let result = ConversationWaitResult {
            target: ConversationWaitTarget {
                conversation_id: "conversation-1".into(),
                label: "build the parser".into(),
                phase: Some(DurableTurnPhase::Completed),
            },
            reply: Some("done".into()),
        };
        assert!(result.target.is_settled());
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::json!({
                "conversationId": "conversation-1",
                "label": "build the parser",
                "phase": "completed",
                "reply": "done"
            })
        );
    }

    #[test]
    fn an_active_phase_is_reported_and_is_never_settled() {
        let target = ConversationWaitTarget {
            conversation_id: "conversation-1".into(),
            label: String::new(),
            phase: Some(DurableTurnPhase::Active),
        };
        assert!(!target.is_settled());
    }

    #[test]
    fn an_excerpt_never_splits_a_code_point() {
        assert_eq!(bounded_excerpt("héllo", 2), "h");
        assert_eq!(bounded_excerpt("hello", 99), "hello");
    }

    #[test]
    fn a_label_must_be_present_and_bounded() {
        assert!(valid_conversation_label("build the parser"));
        assert!(!valid_conversation_label("   "));
        assert!(!valid_conversation_label(&"x".repeat(121)));
    }
}
