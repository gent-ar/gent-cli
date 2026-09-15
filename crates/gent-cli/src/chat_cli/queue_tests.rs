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
    let request = AgentChatIntentFrame::SteerQueuedPrompt {
        request_id: AgentChatRequestId("request".into()),
        receipt_id: ReceiptId("receipt".into()),
        conversation_id: AgentChatConversationId("conversation".into()),
        message_id: "message-1".into(),
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
            let AgentChatIntentFrame::SteerQueuedPrompt {
                request_id,
                receipt_id,
                conversation_id,
                message_id,
            } = read_json_frame(&mut stream).await.unwrap()
            else {
                panic!("expected a steer intent");
            };
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
        true,
    )
    .await
    .unwrap();
    assert_eq!(replies.len(), 2);
    assert_eq!(server.await.unwrap(), ["first", "second"]);
}
