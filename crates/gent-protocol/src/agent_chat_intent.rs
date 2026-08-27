//! Capability-gated future agent-chat intent frames.
//!
//! No runtime currently composes this endpoint. Negotiating these frames alone
//! never starts a provider, creates a conversation, or changes observer mode.

use gent_types::{
    AgentChatConversationId, AgentChatDecisionId, AgentChatDecisionResponse,
    AgentChatPromptDelivery, AgentChatRequestId, AgentChatRunId, AgentChatSelection, ContextPolicy,
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
        /// Raw local path, canonicalized and validated by gentd before it is persisted.
        workspace_path: String,
        selection: AgentChatSelection,
    },
    SendPrompt {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        text: String,
        attachment_ids: Vec<String>,
    },
    QueuePrompt {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        text: String,
        attachment_ids: Vec<String>,
    },
    SendPromptWithTools {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        text: String,
        attachment_ids: Vec<String>,
        tool_source_ids: Vec<String>,
    },
    QueuePromptWithTools {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        text: String,
        attachment_ids: Vec<String>,
        tool_source_ids: Vec<String>,
    },
    SwitchSelection {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        conversation_id: AgentChatConversationId,
        parent_run_id: AgentChatRunId,
        selection: AgentChatSelection,
        context_policy: ContextPolicy,
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
    /// Durable result of selecting a new immutable child run for one conversation.
    Switched {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        conversation_id: AgentChatConversationId,
        parent_run_id: AgentChatRunId,
        run_id: AgentChatRunId,
        context_policy: ContextPolicy,
        context_through_ordinal: u64,
    },
    Accepted {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        /// Canonical conversation resolved while committing the prompt.
        conversation_id: AgentChatConversationId,
        /// Immutable run selected by the ledger while committing the prompt.
        run_id: AgentChatRunId,
        /// Immutable turn created by the same transaction as the receipt.
        turn_id: String,
        /// Durable local delivery state; this never attests that a provider was launched.
        delivery: AgentChatPromptDelivery,
    },
    Interrupted {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
    },
    ForkConversation {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        source_conversation_id: AgentChatConversationId,
        fork_through_message_id: String,
    },
    /// Durable result of forking a conversation's prior messages into a new one.
    Forked {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        source_conversation_id: AgentChatConversationId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
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
        AgentChatConversationId, AgentChatRequestId, AgentChatRunId, ContextPolicy, HostEpoch,
        Receipt, ReceiptId, ReceiptStatus,
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
    fn accepted_prompt_returns_the_ledger_assigned_turn() {
        let frame = AgentChatIntentFrame::Accepted {
            request_id: AgentChatRequestId("request-1".into()),
            receipt: Receipt {
                receipt_id: ReceiptId("receipt-1".into()),
                idempotency_key: "retry-1".into(),
                status: ReceiptStatus::Accepted,
                host_epoch: HostEpoch(1),
            },
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
            turn_id: "turn-1".into(),
            delivery: gent_types::AgentChatPromptDelivery::AwaitingProvider,
        };
        assert_eq!(
            serde_json::to_value(frame).unwrap(),
            json!({
                "type": "accepted",
                "body": {
                    "requestId": "request-1", "receipt": {
                        "receiptId": "receipt-1", "idempotencyKey": "retry-1",
                        "status": "accepted", "hostEpoch": 1
                    },
                    "conversationId": "conversation-1", "runId": "run-1",
                    "turnId": "turn-1", "delivery": "awaitingProvider"
                }
            })
        );
    }

    #[test]
    fn switch_is_receipt_bound_to_an_expected_parent_run() {
        let frame = AgentChatIntentFrame::SwitchSelection {
            request_id: AgentChatRequestId("request-1".into()),
            receipt_id: ReceiptId("receipt-1".into()),
            conversation_id: gent_types::AgentChatConversationId("conversation-1".into()),
            parent_run_id: AgentChatRunId("run-1".into()),
            selection: gent_types::AgentChatSelection {
                provider: gent_types::AgentChatProvider::Codex,
                model: "gpt-5.6".into(),
                effort: gent_types::AgentChatEffort::High,
                mode: gent_types::AgentChatMode::Agent,
            },
            context_policy: ContextPolicy::Preserve,
        };
        assert!(
            serde_json::to_value(frame).unwrap()["body"]
                .get("parentRunId")
                .is_some()
        );
    }

    #[test]
    fn fork_conversation_round_trips_the_source_boundary() {
        let frame = AgentChatIntentFrame::ForkConversation {
            request_id: AgentChatRequestId("request-1".into()),
            receipt_id: gent_types::ReceiptId("receipt-1".into()),
            source_conversation_id: AgentChatConversationId("conversation-1".into()),
            fork_through_message_id: "message-1".into(),
        };
        assert_eq!(
            serde_json::to_value(&frame).unwrap(),
            json!({
                "type": "forkConversation",
                "body": {
                    "requestId": "request-1", "receiptId": "receipt-1",
                    "sourceConversationId": "conversation-1", "forkThroughMessageId": "message-1"
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
