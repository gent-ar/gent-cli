//! Capability-gated future agent-chat intent frames.
//!
//! No runtime currently composes this endpoint. Negotiating these frames alone
//! never starts a provider, creates a conversation, or changes observer mode.

use gent_types::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatDecisionResponse,
    AgentChatPromptDelivery, AgentChatRequestId, AgentChatRunId, AgentChatSelection,
    NormalizedTranscriptEvent, Receipt,
};
use serde::{Deserialize, Serialize};

/// Required before a client may submit future agent-chat intents.
pub const AGENT_CHAT_INTENTS_CAPABILITY: &str = "agent-chat-intents-v1";

/// Future receipt-backed commands plus cursor-based subscription requests.
///
/// Providers, credentials, and provider-native session identifiers never cross
/// this public contract. An authority runtime must still validate every intent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatIntentFrame {
    CreateConversation {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        selection: AgentChatSelection,
    },
    SendPrompt {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        text: String,
    },
    QueuePrompt {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        text: String,
    },
    Interrupt {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
    },
    Decision {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        decision_id: AgentChatDecisionId,
        response: AgentChatDecisionResponse,
    },
    Subscribe {
        request_id: AgentChatRequestId,
        conversation_id: AgentChatConversationId,
        after_cursor: u64,
    },
    /// A cursor-ordered item delivered for one prior subscription request.
    SubscriptionEvent {
        request_id: AgentChatRequestId,
        event: NormalizedTranscriptEvent,
    },
    /// Explicit terminal state for a subscription; a client must resubscribe.
    SubscriptionEnded {
        request_id: AgentChatRequestId,
        reason: AgentChatSubscriptionEnd,
    },
    /// Durable result of creating a conversation and its immutable root run.
    Created {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
    },
    Accepted {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        /// Durable local delivery state; this never attests that a provider was launched.
        delivery: AgentChatPromptDelivery,
    },
}

/// Why a future agent-chat subscription has stopped producing events.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatSubscriptionEnd {
    ResyncRequired,
    ServerClosing,
}

#[cfg(test)]
mod tests {
    use super::{AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, AgentChatSubscriptionEnd};
    use gent_types::{
        AgentChatConversationId, AgentChatRequestId, AgentChatRunId, HostEpoch, Receipt, ReceiptId,
        ReceiptStatus,
    };
    use serde_json::json;

    #[test]
    fn interrupt_has_typed_request_and_receipt_ids() {
        let frame = AgentChatIntentFrame::Interrupt {
            request_id: AgentChatRequestId("request-1".into()),
            receipt_id: ReceiptId("receipt-1".into()),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
        };
        assert_eq!(
            serde_json::to_value(frame).unwrap(),
            json!({
                "type": "interrupt",
                "body": {
                    "requestId": "request-1", "receiptId": "receipt-1",
                    "conversationId": "conversation-1", "runId": "run-1"
                }
            })
        );
        assert_eq!(AGENT_CHAT_INTENTS_CAPABILITY, "agent-chat-intents-v1");
    }

    #[test]
    fn subscribe_binds_a_request_to_one_cursor() {
        let frame = json!({
            "type": "subscribe",
            "body": { "requestId": "request-1", "conversationId": "conversation-1", "afterCursor": 7 }
        });
        assert!(serde_json::from_value::<AgentChatIntentFrame>(frame).is_ok());
    }

    #[test]
    fn subscription_end_is_request_correlated_and_closed() {
        let frame = AgentChatIntentFrame::SubscriptionEnded {
            request_id: AgentChatRequestId("request-1".into()),
            reason: AgentChatSubscriptionEnd::ResyncRequired,
        };
        assert_eq!(
            serde_json::to_value(frame).unwrap(),
            json!({ "type": "subscriptionEnded", "body": { "requestId": "request-1", "reason": "resyncRequired" } })
        );
    }

    #[test]
    fn create_result_returns_only_durable_public_identities() {
        let frame = AgentChatIntentFrame::Created {
            request_id: AgentChatRequestId("request-1".into()),
            receipt: Receipt {
                receipt_id: ReceiptId("receipt-1".into()),
                idempotency_key: "retry-1".into(),
                status: ReceiptStatus::Settled,
                host_epoch: HostEpoch(1),
            },
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
        };
        assert_eq!(
            serde_json::to_value(frame).unwrap(),
            json!({
                "type": "created",
                "body": {
                    "requestId": "request-1", "receipt": {
                        "receiptId": "receipt-1", "idempotencyKey": "retry-1",
                        "status": "settled", "hostEpoch": 1
                    },
                    "conversationId": "conversation-1", "runId": "run-1"
                }
            })
        );
    }

    #[test]
    fn intent_contract_rejects_unknown_fields() {
        let frame = json!({
            "type": "sendPrompt",
            "body": {
                "requestId": "request-1", "receiptId": "receipt-1",
                "conversationId": "conversation-1", "text": "hello",
                "providerSessionId": "never-public"
            }
        });
        assert!(serde_json::from_value::<AgentChatIntentFrame>(frame).is_err());
    }
}
