use serde::{Deserialize, Serialize};

use crate::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatRunId, HostEpoch, PermissionRequest,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PermissionRequestDigest(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionDecisionBinding {
    pub decision_id: AgentChatDecisionId,
    pub request_idempotency_key: String,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub turn_id: String,
    pub policy_id: String,
    pub policy_revision: u64,
    pub host_epoch: HostEpoch,
    pub request_digest_sha256: PermissionRequestDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionDecisionRequest {
    pub binding: PermissionDecisionBinding,
    pub request: PermissionRequest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum PermissionDecisionResponseKind {
    Deny,
    ApproveOnce,
    ApproveExactTool,
    ApproveCategory,
    ApproveCommandPattern,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionDecisionResponse {
    pub binding: PermissionDecisionBinding,
    pub response: PermissionDecisionResponseKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_pattern: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DecisionOutcome {
    Allowed,
    Denied,
    Skipped,
    Cancelled,
}

impl DecisionOutcome {
    #[must_use]
    pub fn answered(response: PermissionDecisionResponseKind, question: bool) -> Self {
        match (response, question) {
            (PermissionDecisionResponseKind::Deny, true) => Self::Skipped,
            (PermissionDecisionResponseKind::Deny, false) => Self::Denied,
            _ => Self::Allowed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PermissionDecisionBinding, PermissionDecisionResponseKind};
    use crate::{
        AgentChatConversationId, AgentChatDecisionId, AgentChatRunId, HostEpoch,
        PermissionRequestDigest,
    };

    #[test]
    fn response_kind_is_closed() {
        assert!(serde_json::from_str::<PermissionDecisionResponseKind>("\"approveOnce\"").is_ok());
        assert!(serde_json::from_str::<PermissionDecisionResponseKind>("\"always\"").is_err());
    }

    #[test]
    fn a_denied_question_is_skipped_and_any_approval_is_allowed() {
        use super::DecisionOutcome;
        use PermissionDecisionResponseKind::{ApproveCategory, ApproveOnce, Deny};
        assert_eq!(
            [
                DecisionOutcome::answered(Deny, true),
                DecisionOutcome::answered(Deny, false),
                DecisionOutcome::answered(ApproveOnce, true),
                DecisionOutcome::answered(ApproveCategory, false),
            ],
            [
                DecisionOutcome::Skipped,
                DecisionOutcome::Denied,
                DecisionOutcome::Allowed,
                DecisionOutcome::Allowed,
            ]
        );
        assert_eq!(
            serde_json::to_value(DecisionOutcome::Cancelled).unwrap(),
            "cancelled"
        );
    }

    #[test]
    fn binding_rejects_unrecognized_client_fields() {
        let binding = PermissionDecisionBinding {
            decision_id: AgentChatDecisionId("decision-1".into()),
            request_idempotency_key: "request-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
            turn_id: "turn-1".into(),
            policy_id: "policy-1".into(),
            policy_revision: 1,
            host_epoch: HostEpoch(1),
            request_digest_sha256: PermissionRequestDigest("a".repeat(64)),
        };
        let mut value = serde_json::to_value(binding).unwrap();
        value["providerSessionId"] = serde_json::json!("forbidden");
        assert!(serde_json::from_value::<PermissionDecisionBinding>(value).is_err());
    }
}
