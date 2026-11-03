use super::{AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, AgentChatSubscriptionEnd};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, ContextPolicy, HostEpoch, Receipt,
    ReceiptId, ReceiptStatus,
};
use serde_json::json;

#[test]
fn interrupt_has_typed_request_and_receipt_ids() {
    let frame = AgentChatIntentFrame::Interrupt {
        request_id: AgentChatRequestId("request-1".into()),
        receipt_id: ReceiptId("receipt-1".into()),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        run_id: AgentChatRunId("run-1".into()),
        expected_turn_id: None,
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
fn interrupt_turn_binding_is_optional_and_round_trips() {
    for expected in [None, Some("turn-1")] {
        let mut wire = json!({"type":"interrupt", "body": {
            "requestId":"request-1", "receiptId":"receipt-1",
            "conversationId":"conversation-1", "runId":"run-1"
        }});
        if let Some(turn) = expected {
            wire["body"]["expectedTurnId"] = turn.into();
        }
        let parsed: AgentChatIntentFrame = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
    }
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
        message_id: "message-1".into(),
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
                "turnId": "turn-1", "messageId": "message-1",
                "delivery": "awaitingProvider"
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
fn queued_prompt_cancellation_is_receipt_bound_to_one_message() {
    let request = json!({
        "type": "cancelQueuedPrompt",
        "body": {
            "requestId": "request-1", "receiptId": "receipt-1",
            "conversationId": "conversation-1", "messageId": "message-1"
        }
    });
    assert!(matches!(
        serde_json::from_value::<AgentChatIntentFrame>(request).unwrap(),
        AgentChatIntentFrame::CancelQueuedPrompt { message_id, .. } if message_id == "message-1"
    ));
    let reply = AgentChatIntentFrame::QueuedPromptCanceled {
        request_id: AgentChatRequestId("request-1".into()),
        receipt: Receipt {
            receipt_id: ReceiptId("receipt-1".into()),
            idempotency_key: "agent-chat-cancel-queued-prompt:receipt-1".into(),
            status: ReceiptStatus::Settled,
            host_epoch: HostEpoch(1),
        },
        conversation_id: AgentChatConversationId("conversation-1".into()),
        message_id: "message-1".into(),
    };
    assert_eq!(
        serde_json::to_value(reply).unwrap(),
        json!({
            "type": "queuedPromptCanceled",
            "body": {
                "requestId": "request-1", "receipt": {
                    "receiptId": "receipt-1",
                    "idempotencyKey": "agent-chat-cancel-queued-prompt:receipt-1",
                    "status": "settled", "hostEpoch": 1
                },
                "conversationId": "conversation-1", "messageId": "message-1"
            }
        })
    );
}

#[test]
fn queued_prompt_steer_is_receipt_bound_to_one_message() {
    let request = json!({
        "type": "steerQueuedPrompt",
        "body": {
            "requestId": "request-1", "receiptId": "receipt-1",
            "conversationId": "conversation-1", "messageId": "message-1"
        }
    });
    assert!(matches!(
        serde_json::from_value::<AgentChatIntentFrame>(request).unwrap(),
        AgentChatIntentFrame::SteerQueuedPrompt { message_id, .. } if message_id == "message-1"
    ));
    let reply = AgentChatIntentFrame::QueuedPromptSteered {
        request_id: AgentChatRequestId("request-1".into()),
        receipt: Receipt {
            receipt_id: ReceiptId("receipt-1".into()),
            idempotency_key: "agent-chat-steer-queued-prompt:message-1".into(),
            status: ReceiptStatus::Settled,
            host_epoch: HostEpoch(1),
        },
        conversation_id: AgentChatConversationId("conversation-1".into()),
        message_id: "message-1".into(),
    };
    assert_eq!(
        serde_json::to_value(reply).unwrap(),
        json!({
            "type": "queuedPromptSteered",
            "body": {
                "requestId": "request-1", "receipt": {
                    "receiptId": "receipt-1",
                    "idempotencyKey": "agent-chat-steer-queued-prompt:message-1",
                    "status": "settled", "hostEpoch": 1
                },
                "conversationId": "conversation-1", "messageId": "message-1"
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
