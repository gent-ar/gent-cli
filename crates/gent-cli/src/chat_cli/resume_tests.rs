use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, Negotiated, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatPromptDelivery, CapabilitySet, HostEpoch, PROTOCOL_MAX, Receipt, ReceiptStatus,
};
use tokio::net::UnixListener;

use super::{ChatCommand, execute};

#[tokio::test]
async fn resume_submits_to_the_existing_gent_conversation() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
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
        let AgentChatIntentFrame::SendPrompt {
            request_id,
            receipt_id,
            conversation_id,
            text,
        } = read_json_frame(&mut stream).await.unwrap()
        else {
            panic!("expected resumed prompt");
        };
        assert_eq!(conversation_id.0, "conversation-1");
        assert_eq!(text, "continue from the selected run");
        write_json_frame(
            &mut stream,
            &AgentChatIntentFrame::Accepted {
                request_id,
                receipt: Receipt {
                    receipt_id,
                    idempotency_key: "redacted".into(),
                    status: ReceiptStatus::Accepted,
                    host_epoch: HostEpoch(1),
                },
                conversation_id,
                run_id: gent_types::AgentChatRunId("run-2".into()),
                turn_id: "turn-2".into(),
                delivery: AgentChatPromptDelivery::AwaitingProvider,
            },
        )
        .await
        .unwrap();
    });
    let reply = execute(
        Some(directory.path().into()),
        true,
        ChatCommand::Resume(super::resume::ResumeArgs {
            conversation_id: "conversation-1".into(),
            text: "continue from the selected run".into(),
            request_id: Some("request-1".into()),
            receipt_id: Some("receipt-1".into()),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply,
        AgentChatIntentFrame::Accepted { conversation_id, run_id, turn_id, .. }
            if conversation_id.0 == "conversation-1" && run_id.0 == "run-2" && turn_id == "turn-2"
    ));
}
