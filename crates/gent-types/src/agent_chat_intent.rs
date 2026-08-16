//! Typed future agent-chat intents. These are values only; they have no executor.

use serde::{Deserialize, Serialize};

/// Client-generated correlation identifier for an agent-chat request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentChatRequestId(pub String);

/// A durable conversation identity used by a future agent-chat command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentChatConversationId(pub String);

/// A durable run identity used by a future agent-chat interrupt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentChatRunId(pub String);

/// A durable decision identity used by a future agent-chat decision response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentChatDecisionId(pub String);

/// A user-selected response to an already-presented decision.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatDecisionResponse {
    ApproveOnce,
    ApproveAlways,
    Deny,
}

#[cfg(test)]
mod tests {
    use super::{AgentChatDecisionResponse, AgentChatRequestId};

    #[test]
    fn intent_identifiers_are_json_scalars() {
        assert_eq!(
            serde_json::to_string(&AgentChatRequestId("request-1".into())).unwrap(),
            "\"request-1\""
        );
    }

    #[test]
    fn decision_response_is_a_closed_contract() {
        assert!(serde_json::from_str::<AgentChatDecisionResponse>("\"later\"").is_err());
    }
}
