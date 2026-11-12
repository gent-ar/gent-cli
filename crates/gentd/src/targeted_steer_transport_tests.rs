use super::*;
use gent_protocol::{AGENT_CHAT_STEER_TURN_CAPABILITY, WireFrame, read_frame, read_json_frame};
use gent_types::{HostEpoch, Receipt, ReceiptStatus};
use serde_json::json;
use tokio::io::duplex;
#[path = "atomic_steer_transport_tests.rs"]
mod atomic;

struct Port;
impl IntentPort for Port {
    fn exchange(
        &self,
        frame: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        let AgentChatIntentFrame::SteerQueuedPromptForTurn {
            request_id,
            receipt_id,
            conversation_id,
            message_id,
            expected_run_id,
            expected_turn_id,
        } = frame
        else {
            panic!("targeted request expected")
        };
        assert_eq!(expected_run_id.0, "run-original");
        assert_eq!(expected_turn_id, "turn-original");
        Ok(vec![AgentChatIntentFrame::QueuedPromptSteered {
            request_id,
            conversation_id,
            message_id,
            receipt: Receipt {
                receipt_id,
                idempotency_key: "target-bound-key".into(),
                status: ReceiptStatus::Settled,
                host_epoch: HostEpoch(1),
            },
        }])
    }
}

fn request() -> Value {
    json!({"type":"steerQueuedPromptForTurn","body":{"requestId":"steer","receiptId":"receipt","conversationId":"conversation","messageId":"queued","expectedRunId":"run-original","expectedTurnId":"turn-original"}})
}

#[tokio::test]
async fn targeted_steer_is_capability_gated_before_authority() {
    let (mut reader, mut writer) = duplex(4096);
    let caps = CapabilitySet(vec![AGENT_CHAT_INTENTS_CAPABILITY.into()]);
    assert!(
        dispatch_port(&mut writer, &Port, &caps, &request())
            .await
            .unwrap()
    );
    assert!(
        matches!(read_frame(&mut reader).await.unwrap(),WireFrame::Error {code,..} if code=="unsupportedCapability")
    );
}

#[tokio::test]
async fn targeted_steer_wire_preserves_target_and_matching_receipt() {
    let (mut reader, mut writer) = duplex(4096);
    let caps = CapabilitySet(vec![
        AGENT_CHAT_INTENTS_CAPABILITY.into(),
        AGENT_CHAT_STEER_TURN_CAPABILITY.into(),
    ]);
    assert!(
        dispatch_port(&mut writer, &Port, &caps, &request())
            .await
            .unwrap()
    );
    assert!(
        matches!(read_json_frame::<_,AgentChatIntentFrame>(&mut reader).await.unwrap(),AgentChatIntentFrame::QueuedPromptSteered {receipt,message_id,..} if receipt.receipt_id.0=="receipt" && message_id=="queued")
    );
    let mut missing = request();
    missing["body"]
        .as_object_mut()
        .unwrap()
        .remove("expectedTurnId");
    assert!(serde_json::from_value::<AgentChatIntentFrame>(missing).is_err());
}

#[test]
fn advertised_native_steer_capabilities_are_provider_scoped() {
    let enabled = gent_runtime::catalog::declared_capabilities_with_agent_chat(true).0;
    assert!(enabled.contains(&AGENT_CHAT_STEER_TURN_CAPABILITY.into()));
    assert!(enabled.contains(&gent_protocol::AGENT_CHAT_STEER_ENQUEUE_CAPABILITY.into()));
    for provider in ["claude", "codex", "claurst"] {
        assert!(enabled.contains(&format!("agent-chat-steer-turn-{provider}-v1")));
    }
    assert!(
        !gent_runtime::catalog::declared_capabilities()
            .0
            .contains(&AGENT_CHAT_STEER_TURN_CAPABILITY.into())
    );
}
