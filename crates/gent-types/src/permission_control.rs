//! Immutable, provider-neutral permission-control values.

use serde::{Deserialize, Serialize};

use crate::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatRunId, HostEpoch, PermissionRequest,
};

/// Opaque SHA-256 identity of the exact normalized request presented to a user.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct PermissionRequestDigest(pub String);

/// Immutable scope shared by a permission request and the response to it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionDecisionBinding {
    pub decision_id: AgentChatDecisionId,
    /// Client retry identity; a future ledger binds one response to this exact request.
    pub request_idempotency_key: String,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub turn_id: String,
    pub policy_id: String,
    pub policy_revision: u64,
    pub host_epoch: HostEpoch,
    pub request_digest_sha256: PermissionRequestDigest,
}

/// One immutable permission request that a client may render, but never execute.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionDecisionRequest {
    pub binding: PermissionDecisionBinding,
    pub request: PermissionRequest,
}

/// The closed user response to one exact permission request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum PermissionDecisionResponseKind {
    Deny,
    ApproveOnce,
    ApproveExactTool,
    ApproveCategory,
}

/// An immutable response that must echo the request's complete durable binding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionDecisionResponse {
    pub binding: PermissionDecisionBinding,
    pub response: PermissionDecisionResponseKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
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
