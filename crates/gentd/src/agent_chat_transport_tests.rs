use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, AgentChatSubscriptionEnd, WireFrame,
    read_frame, read_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, CapabilitySet, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, Receipt, ReceiptId, ReceiptStatus,
};
use serde_json::json;
use tokio::io::duplex;

use crate::agent_chat_transport::{IntentPort, dispatch_port};

#[derive(Clone)]
struct FakePort;

impl IntentPort for FakePort {
    fn exchange(&self, request: AgentChatIntentFrame) -> Result<Vec<AgentChatIntentFrame>, String> {
        match request {
            AgentChatIntentFrame::CreateConversation {
                request_id,
                receipt_id,
                ..
            } => Ok(vec![AgentChatIntentFrame::Created {
                request_id,
                receipt: receipt(receipt_id),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: gent_types::AgentChatRunId("run-1".into()),
            }]),
            AgentChatIntentFrame::SendPrompt {
                request_id,
                receipt_id,
                ..
            } => Ok(vec![accepted(request_id, receipt_id)]),
            AgentChatIntentFrame::SwitchSelection {
                request_id,
                receipt_id,
                conversation_id,
                parent_run_id,
                ..
            } => Ok(vec![AgentChatIntentFrame::Switched {
                request_id,
                receipt: receipt(receipt_id),
                conversation_id,
                parent_run_id,
                run_id: gent_types::AgentChatRunId("run-2".into()),
                context_through_ordinal: 3,
            }]),
            AgentChatIntentFrame::Subscribe { request_id, .. } => Ok(vec![
                AgentChatIntentFrame::SubscriptionEvent {
                    request_id: request_id.clone(),
                    event: NormalizedTranscriptEvent {
                        cursor: 2,
                        event_id: "event-2".into(),
                        turn_id: "turn-1".into(),
                        run_id: "run-1".into(),
                        kind: NormalizedTranscriptKind::AssistantMessage,
                        text: "hello".into(),
                        is_partial: false,
                    },
                },
                AgentChatIntentFrame::SubscriptionEnded {
                    request_id,
                    reason: AgentChatSubscriptionEnd::ServerClosing,
                },
            ]),
            _ => Err("observer-disabled".into()),
        }
    }
}

#[derive(Clone)]
struct BadReceiptPort;

impl IntentPort for BadReceiptPort {
    fn exchange(&self, _: AgentChatIntentFrame) -> Result<Vec<AgentChatIntentFrame>, String> {
        Ok(vec![accepted(
            AgentChatRequestId("wrong-request".into()),
            ReceiptId("wrong-receipt".into()),
        )])
    }
}

#[derive(Clone)]
struct ObserverPort;

impl IntentPort for ObserverPort {
    fn exchange(&self, _: AgentChatIntentFrame) -> Result<Vec<AgentChatIntentFrame>, String> {
        Err("observer-disabled".into())
    }
}

fn accepted(request_id: AgentChatRequestId, receipt_id: ReceiptId) -> AgentChatIntentFrame {
    AgentChatIntentFrame::Accepted {
        request_id,
        receipt: receipt(receipt_id),
        delivery: gent_types::AgentChatPromptDelivery::AwaitingProvider,
    }
}

fn receipt(receipt_id: ReceiptId) -> Receipt {
    Receipt {
        receipt_id,
        idempotency_key: "retry-1".into(),
        status: ReceiptStatus::Accepted,
        host_epoch: gent_types::HostEpoch(1),
    }
}

fn capabilities() -> CapabilitySet {
    CapabilitySet(vec![AGENT_CHAT_INTENTS_CAPABILITY.into()])
}

fn prompt() -> serde_json::Value {
    json!({ "type": "sendPrompt", "body": { "requestId": "request-1", "receiptId": "receipt-1", "conversationId": "conversation-1", "text": "hello" } })
}

#[tokio::test]
async fn prompt_reply_requires_a_correlated_receipt() {
    let (mut reader, mut writer) = duplex(4096);
    assert!(
        dispatch_port(&mut writer, &FakePort, &capabilities(), &prompt())
            .await
            .unwrap()
    );
    assert!(
        matches!(read_json_frame::<_, AgentChatIntentFrame>(&mut reader).await.unwrap(), AgentChatIntentFrame::Accepted { request_id, receipt, delivery: gent_types::AgentChatPromptDelivery::AwaitingProvider } if request_id.0 == "request-1" && receipt.receipt_id.0 == "receipt-1")
    );
}

#[tokio::test]
async fn create_reply_includes_durable_conversation_and_run_identities() {
    let (mut reader, mut writer) = duplex(4096);
    let request = json!({ "type": "createConversation", "body": { "requestId": "create-1", "receiptId": "receipt-1", "selection": { "provider": "claude", "model": "haiku", "effort": "low", "mode": "ask" } } });
    assert!(
        dispatch_port(&mut writer, &FakePort, &capabilities(), &request)
            .await
            .unwrap()
    );
    assert!(
        matches!(read_json_frame::<_, AgentChatIntentFrame>(&mut reader).await.unwrap(), AgentChatIntentFrame::Created { conversation_id, run_id, .. } if conversation_id.0 == "conversation-1" && run_id.0 == "run-1")
    );
}

#[tokio::test]
async fn switch_reply_binds_the_expected_parent_and_new_child_run() {
    let (mut reader, mut writer) = duplex(4096);
    let request = json!({ "type": "switchSelection", "body": {
        "requestId": "switch-1", "receiptId": "receipt-1", "conversationId": "conversation-1",
        "parentRunId": "run-1", "selection": { "provider": "codex", "model": "gpt-5.6", "effort": "high", "mode": "agent" }
    } });
    assert!(
        dispatch_port(&mut writer, &FakePort, &capabilities(), &request)
            .await
            .unwrap()
    );
    assert!(matches!(
        read_json_frame::<_, AgentChatIntentFrame>(&mut reader).await.unwrap(),
        AgentChatIntentFrame::Switched { parent_run_id, run_id, context_through_ordinal, .. }
            if parent_run_id.0 == "run-1" && run_id.0 == "run-2" && context_through_ordinal == 3
    ));
}

#[tokio::test]
async fn mismatched_receipt_is_rejected_before_it_reaches_the_client() {
    let (mut reader, mut writer) = duplex(4096);
    assert!(
        dispatch_port(&mut writer, &BadReceiptPort, &capabilities(), &prompt())
            .await
            .unwrap()
    );
    assert!(
        matches!(read_frame(&mut reader).await.unwrap(), WireFrame::Error { code, .. } if code == "invalidAgentChatResponse")
    );
}

#[tokio::test]
async fn subscription_reply_is_cursor_ordered_and_explicitly_terminal() {
    let (mut reader, mut writer) = duplex(4096);
    let request = json!({ "type": "subscribe", "body": { "requestId": "request-2", "conversationId": "conversation-1", "afterCursor": 1 } });
    assert!(
        dispatch_port(&mut writer, &FakePort, &capabilities(), &request)
            .await
            .unwrap()
    );
    assert!(
        matches!(read_json_frame::<_, AgentChatIntentFrame>(&mut reader).await.unwrap(), AgentChatIntentFrame::SubscriptionEvent { event, .. } if event.cursor == 2)
    );
    assert!(matches!(
        read_json_frame::<_, AgentChatIntentFrame>(&mut reader)
            .await
            .unwrap(),
        AgentChatIntentFrame::SubscriptionEnded { .. }
    ));
}

#[tokio::test]
async fn absent_capability_leaves_the_frame_for_the_generic_rejection_path() {
    let (_, mut writer) = duplex(4096);
    assert!(
        !dispatch_port(&mut writer, &FakePort, &CapabilitySet::default(), &prompt())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn observer_port_error_is_a_protocol_error_without_a_provider_effect() {
    let (mut reader, mut writer) = duplex(4096);
    let request = json!({ "type": "createConversation", "body": { "requestId": "r", "receiptId": "x", "selection": { "provider": "claude", "model": "haiku", "effort": "low", "mode": "ask" } } });
    assert!(
        dispatch_port(&mut writer, &ObserverPort, &capabilities(), &request)
            .await
            .unwrap()
    );
    assert!(
        matches!(read_frame(&mut reader).await.unwrap(), WireFrame::Error { code, message } if code == "agentChatRejected" && message == "observer-disabled")
    );
}

#[test]
fn prompt_fixture_has_one_conversation_identity() {
    assert!(serde_json::from_value::<AgentChatIntentFrame>(prompt()).is_ok());
    assert_eq!(
        AgentChatConversationId("conversation-1".into()).0,
        "conversation-1"
    );
}
