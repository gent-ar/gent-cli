use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, Negotiated, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, CapabilitySet, HostEpoch, PROTOCOL_MAX, Receipt,
    ReceiptId, ReceiptStatus,
};
use tokio::net::UnixListener;

use super::{deliver, valid_reply};

#[path = "queue_cli_tests.rs"]
mod cli;

fn target() -> gent_types::ConversationRunStatus {
    gent_types::ConversationRunStatus {
        run_id: "running-run".into(),
        parent_run_id: None,
        provider: "codex".into(),
        active_turn_id: Some("observed-turn".into()),
        live_status: None,
    }
}

fn capabilities() -> CapabilitySet {
    CapabilitySet(vec![
        AGENT_CHAT_INTENTS_CAPABILITY.into(),
        gent_protocol::AGENT_CHAT_STEER_TURN_CAPABILITY.into(),
        "agent-chat-steer-turn-codex-v1".into(),
    ])
}

fn receipt(receipt_id: ReceiptId) -> Receipt {
    Receipt {
        receipt_id,
        idempotency_key: "key".into(),
        status: ReceiptStatus::Settled,
        host_epoch: HostEpoch(1),
    }
}

#[test]
fn steer_and_cancel_replies_must_echo_the_exact_message() {
    let request = AgentChatIntentFrame::SteerQueuedPromptForTurn {
        request_id: AgentChatRequestId("request".into()),
        receipt_id: ReceiptId("receipt".into()),
        conversation_id: AgentChatConversationId("conversation".into()),
        message_id: "message-1".into(),
        expected_run_id: gent_types::AgentChatRunId("running-run".into()),
        expected_turn_id: "observed-turn".into(),
    };
    let reply = |message_id: &str| AgentChatIntentFrame::QueuedPromptSteered {
        request_id: AgentChatRequestId("request".into()),
        receipt: receipt(ReceiptId("receipt".into())),
        conversation_id: AgentChatConversationId("conversation".into()),
        message_id: message_id.into(),
    };
    assert_eq!(valid_reply(&request, &reply("message-1")), Some(true));
    assert_eq!(valid_reply(&request, &reply("message-2")), Some(false));
    let canceled = AgentChatIntentFrame::QueuedPromptCanceled {
        request_id: AgentChatRequestId("request".into()),
        receipt: receipt(ReceiptId("receipt".into())),
        conversation_id: AgentChatConversationId("conversation".into()),
        message_id: "message-1".into(),
    };
    assert_eq!(valid_reply(&request, &canceled), Some(false));
}

#[tokio::test]
async fn steering_sends_each_queued_prompt_oldest_first_through_gentd() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        let mut steered = Vec::new();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let WireFrame::Hello(hello) = read_frame(&mut stream).await.unwrap() else {
                panic!("expected hello");
            };
            for capability in capabilities().0 {
                assert!(hello.capabilities.0.contains(&capability));
            }
            write_frame(
                &mut stream,
                &WireFrame::Negotiated(Negotiated {
                    protocol: PROTOCOL_MAX,
                    capabilities: capabilities(),
                }),
            )
            .await
            .unwrap();
            let AgentChatIntentFrame::SteerQueuedPromptForTurn {
                request_id,
                receipt_id,
                conversation_id,
                message_id,
                expected_run_id,
                expected_turn_id,
            } = read_json_frame(&mut stream).await.unwrap()
            else {
                panic!("expected a steer intent");
            };
            assert_eq!(expected_run_id.0, "running-run");
            assert_eq!(expected_turn_id, "observed-turn");
            steered.push(message_id.clone());
            write_json_frame(
                &mut stream,
                &AgentChatIntentFrame::QueuedPromptSteered {
                    request_id,
                    receipt: receipt(receipt_id),
                    conversation_id,
                    message_id,
                },
            )
            .await
            .unwrap();
        }
        steered
    });
    let replies = deliver(
        Some(directory.path().into()),
        true,
        "conversation-1",
        vec!["first".into(), "second".into()],
        Some(&target()),
    )
    .await
    .unwrap();
    assert_eq!(replies.len(), 2);
    assert_eq!(server.await.unwrap(), ["first", "second"]);
}

#[tokio::test]
async fn steering_requires_generic_and_exact_provider_capabilities_before_writing() {
    for provider in ["claude", "codex", "claurst"] {
        for missing in [gent_protocol::AGENT_CHAT_STEER_TURN_CAPABILITY, "provider"] {
            let directory = tempfile::tempdir().unwrap();
            let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
            let mut supported = capabilities();
            supported.0[2] = format!("agent-chat-steer-turn-{provider}-v1");
            if missing == "provider" {
                supported.0[2] = "agent-chat-steer-turn-other-v1".into();
            } else {
                supported.0.retain(|item| item != missing);
            }
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let _ = read_frame(&mut stream).await.unwrap();
                write_frame(
                    &mut stream,
                    &WireFrame::Negotiated(Negotiated {
                        protocol: PROTOCOL_MAX,
                        capabilities: supported,
                    }),
                )
                .await
                .unwrap();
                assert!(
                    read_json_frame::<_, serde_json::Value>(&mut stream)
                        .await
                        .is_err()
                );
            });
            let mut observed = target();
            observed.provider = provider.into();
            let error = deliver(
                Some(directory.path().into()),
                true,
                "conversation-1",
                vec!["first".into()],
                Some(&observed),
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains("upgrade gentd"));
            server.await.unwrap();
        }
    }
}

#[tokio::test]
async fn stale_target_rejection_stops_the_batch_without_retargeting() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: capabilities(),
            }),
        )
        .await
        .unwrap();
        assert!(matches!(read_json_frame(&mut stream).await.unwrap(),
            AgentChatIntentFrame::SteerQueuedPromptForTurn {
                expected_run_id, expected_turn_id, message_id, ..
            } if expected_run_id.0 == "running-run" && expected_turn_id == "observed-turn"
                && message_id == "first"));
        write_frame(
            &mut stream,
            &WireFrame::Error {
                code: "queuedPromptNotSteerable".into(),
                message: "target ended".into(),
            },
        )
        .await
        .unwrap();
        listener
    });
    let error = deliver(
        Some(directory.path().into()),
        true,
        "conversation-1",
        vec!["first".into(), "second".into()],
        Some(&target()),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("queuedPromptNotSteerable"));
    let listener = server.await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancel_needs_no_turn_or_steering_capability() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![AGENT_CHAT_INTENTS_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        let AgentChatIntentFrame::CancelQueuedPrompt {
            request_id,
            receipt_id,
            conversation_id,
            message_id,
        } = read_json_frame(&mut stream).await.unwrap()
        else {
            panic!("expected cancel");
        };
        write_json_frame(
            &mut stream,
            &AgentChatIntentFrame::QueuedPromptCanceled {
                request_id,
                receipt: receipt(receipt_id),
                conversation_id,
                message_id,
            },
        )
        .await
        .unwrap();
    });
    assert_eq!(
        deliver(
            Some(directory.path().into()),
            true,
            "conversation-1",
            vec!["first".into()],
            None
        )
        .await
        .unwrap()
        .len(),
        1
    );
    server.await.unwrap();
}
