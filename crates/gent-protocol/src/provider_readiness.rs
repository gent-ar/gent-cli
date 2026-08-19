//! Capability-gated provider readiness frames for one exact Gent chat selection.

use gent_types::{AgentChatConversationId, AgentChatProvider, AgentChatRunId};
use serde::{Deserialize, Serialize};

use crate::ProviderInstallReview;

/// Required before a client may ask Gent to assess one selected provider run.
pub const PROVIDER_READINESS_CAPABILITY: &str = "provider-readiness-v2";

/// Server-owned reason why readiness cannot produce a public install review.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderReadinessUnavailable {
    ClaurstPrivateBridge,
    ProvenanceUnreadable,
}

/// Server-owned classification for a daemon-generated install review.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderReadinessReviewState {
    MissingInstall,
    InvalidInstallation,
}

/// Request/reply frames for readiness of the exact durable current run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProviderReadinessFrame {
    Assess {
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
    },
    Ready {
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        provider: AgentChatProvider,
    },
    Review {
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        state: ProviderReadinessReviewState,
        review: ProviderInstallReview,
    },
    Unavailable {
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
        reason: ProviderReadinessUnavailable,
    },
}

#[cfg(test)]
mod tests {
    use super::{PROVIDER_READINESS_CAPABILITY, ProviderReadinessFrame};
    use serde_json::json;

    #[test]
    fn readiness_request_has_no_client_provider_or_plan() {
        assert_eq!(PROVIDER_READINESS_CAPABILITY, "provider-readiness-v2");
        assert!(
            serde_json::from_value::<ProviderReadinessFrame>(json!({
                "type": "assess", "body": { "conversationId": "c", "runId": "r" }
            }))
            .is_ok()
        );
        assert!(serde_json::from_value::<ProviderReadinessFrame>(json!({
            "type": "assess", "body": { "conversationId": "c", "runId": "r", "provider": "codex" }
        })).is_err());
    }
}
