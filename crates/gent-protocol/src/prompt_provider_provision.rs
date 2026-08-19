//! Capability-gated consent frames for installing a provider required by one held prompt.

use gent_types::{AgentChatConversationId, AgentChatRunId, HostEpoch, Receipt, ReceiptId};
use serde::{Deserialize, Serialize};

/// Required before a client may confirm a daemon-derived prompt-provider install review.
pub const PROMPT_PROVIDER_PROVISION_CAPABILITY: &str = "prompt-provider-provision-v1";

/// Daemon-owned result of a prompt-provider provision confirmation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptProviderProvisionState {
    Completed,
    ConsentRequired,
    Failed,
    PlanMismatch,
    Unprovable,
}

/// Request/reply frames for the one exact prompt a daemon readiness review placed on hold.
///
/// The request deliberately excludes provider, action, package, executable, plan, and policy.
/// Gent derives them from the durable selected run and its dependency catalog.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PromptProviderProvisionFrame {
    Confirm {
        receipt_id: ReceiptId,
        idempotency_key: String,
        host_epoch: HostEpoch,
        prompt_receipt_id: ReceiptId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        consent_granted: bool,
        reviewed_plan_digest: String,
    },
    Result {
        receipt: Receipt,
        prompt_receipt_id: ReceiptId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        state: PromptProviderProvisionState,
    },
}

#[cfg(test)]
mod tests {
    use super::{PROMPT_PROVIDER_PROVISION_CAPABILITY, PromptProviderProvisionFrame};
    use serde_json::json;

    #[test]
    fn confirmation_cannot_inject_provider_native_or_plan_values() {
        assert_eq!(
            PROMPT_PROVIDER_PROVISION_CAPABILITY,
            "prompt-provider-provision-v1"
        );
        assert!(
            serde_json::from_value::<PromptProviderProvisionFrame>(json!({
                "type": "confirm", "body": {
                    "receiptId": "receipt", "idempotencyKey": "key", "hostEpoch": 1,
                    "promptReceiptId": "prompt", "conversationId": "conversation", "runId": "run",
                    "consentGranted": true, "reviewedPlanDigest": "a".repeat(64)
                }
            }))
            .is_ok()
        );
        assert!(
            serde_json::from_value::<PromptProviderProvisionFrame>(json!({
                "type": "confirm", "body": {
                    "receiptId": "receipt", "idempotencyKey": "key", "hostEpoch": 1,
                    "promptReceiptId": "prompt", "conversationId": "conversation", "runId": "run",
                    "consentGranted": true, "reviewedPlanDigest": "a".repeat(64),
                    "provider": "codex"
                }
            }))
            .is_err()
        );
    }
}
