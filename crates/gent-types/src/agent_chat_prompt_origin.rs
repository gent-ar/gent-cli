use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentChatPromptOrigin {
    User,
    GoalContinuation {
        goal_id: String,
        continues_after_ordinal: u64,
    },
}

impl AgentChatPromptOrigin {
    #[must_use]
    pub fn goal_id(&self) -> Option<&str> {
        match self {
            Self::User => None,
            Self::GoalContinuation { goal_id, .. } => Some(goal_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::AgentChatPromptOrigin;

    #[test]
    fn prompt_origin_is_a_kind_tagged_value() {
        assert_eq!(
            serde_json::to_value(AgentChatPromptOrigin::User).unwrap(),
            json!({"kind": "user"})
        );
        let continuation = AgentChatPromptOrigin::GoalContinuation {
            goal_id: "goal-1".into(),
            continues_after_ordinal: 7,
        };
        assert_eq!(
            serde_json::to_value(&continuation).unwrap(),
            json!({"kind": "goalContinuation", "goalId": "goal-1", "continuesAfterOrdinal": 7})
        );
        assert_eq!(continuation.goal_id(), Some("goal-1"));
        assert_eq!(AgentChatPromptOrigin::User.goal_id(), None);
    }
}
