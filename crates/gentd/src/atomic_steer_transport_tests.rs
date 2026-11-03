use super::*;

struct AtomicPort {
    wrong_run: bool,
}
impl IntentPort for AtomicPort {
    fn exchange(
        &self,
        frame: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        let AgentChatIntentFrame::SteerPromptForTurn(input) = frame else {
            panic!("atomic primary required")
        };
        assert_eq!(input.expected_turn_id, "original-turn");
        assert_eq!(input.expected_run_id.0, "original-run");
        assert!(input.attachment_ids.is_empty());
        assert!(input.tool_source_ids.is_empty());
        Ok(vec![AgentChatIntentFrame::Accepted {
            request_id: input.request_id,
            receipt: Receipt {
                receipt_id: input.receipt_id,
                idempotency_key: "atomic-targeted".into(),
                status: ReceiptStatus::Settled,
                host_epoch: HostEpoch(1),
            },
            conversation_id: input.conversation_id,
            run_id: gent_types::AgentChatRunId(
                if self.wrong_run {
                    "wrong-run"
                } else {
                    "original-run"
                }
                .into(),
            ),
            turn_id: "queued-future-turn".into(),
            message_id: "same-stable-message".into(),
            delivery: gent_types::AgentChatPromptDelivery::Queued,
        }])
    }
}

fn atomic_request() -> Value {
    json!({"type":"steerPromptForTurn","body":{"requestId":"primary","receiptId":"primary-receipt","conversationId":"conversation",
        "expectedRunId":"original-run","expectedTurnId":"original-turn","text":"user explicit steer"}})
}

fn atomic_caps(include_enqueue: bool) -> CapabilitySet {
    let mut caps = vec![
        AGENT_CHAT_INTENTS_CAPABILITY.into(),
        AGENT_CHAT_STEER_TURN_CAPABILITY.into(),
        "agent-chat-steer-turn-codex-v1".into(),
    ];
    if include_enqueue {
        caps.push(gent_protocol::AGENT_CHAT_STEER_ENQUEUE_CAPABILITY.into());
    }
    CapabilitySet(caps)
}

#[tokio::test]
async fn primary_atomic_enqueue_requires_its_distinct_capability_before_authority() {
    let (mut reader, mut writer) = duplex(4096);
    assert!(
        dispatch_port(
            &mut writer,
            &AtomicPort { wrong_run: false },
            &atomic_caps(false),
            &atomic_request()
        )
        .await
        .unwrap()
    );
    assert!(
        matches!(read_frame(&mut reader).await.unwrap(),WireFrame::Error {code,..} if code=="unsupportedCapability")
    );
}

#[tokio::test]
async fn primary_atomic_wire_accepts_defaults_and_returns_the_same_receipt_and_queued_identity() {
    let (mut reader, mut writer) = duplex(4096);
    assert!(
        dispatch_port(
            &mut writer,
            &AtomicPort { wrong_run: false },
            &atomic_caps(true),
            &atomic_request()
        )
        .await
        .unwrap()
    );
    assert!(
        matches!(read_json_frame::<_,AgentChatIntentFrame>(&mut reader).await.unwrap(),AgentChatIntentFrame::Accepted {receipt,message_id,run_id,..}
        if receipt.receipt_id.0=="primary-receipt" && message_id=="same-stable-message" && run_id.0=="original-run")
    );
    let mut missing = atomic_request();
    missing["body"]
        .as_object_mut()
        .unwrap()
        .remove("expectedTurnId");
    assert!(serde_json::from_value::<AgentChatIntentFrame>(missing).is_err());
}

#[tokio::test]
async fn primary_atomic_reply_cannot_substitute_a_newer_run() {
    let (mut reader, mut writer) = duplex(4096);
    assert!(
        dispatch_port(
            &mut writer,
            &AtomicPort { wrong_run: true },
            &atomic_caps(true),
            &atomic_request()
        )
        .await
        .unwrap()
    );
    assert!(
        matches!(read_frame(&mut reader).await.unwrap(),WireFrame::Error {code,..} if code=="invalidAgentChatResponse")
    );
}
