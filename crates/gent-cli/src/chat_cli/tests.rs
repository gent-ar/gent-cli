use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, Hello, Negotiated, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, CapabilitySet, HostEpoch, PROTOCOL_MAX, Receipt, ReceiptStatus,
};
use tokio::net::UnixListener;

use super::{ChatCommand, CreateArgs, Effort, Mode, Provider, execute};

#[tokio::test]
async fn create_negotiates_agent_chat_and_requires_a_matching_created_reply() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(Hello { capabilities, .. })
                if capabilities.0.iter().any(|item| item == AGENT_CHAT_INTENTS_CAPABILITY)
        ));
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![AGENT_CHAT_INTENTS_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        let AgentChatIntentFrame::CreateConversation {
            request_id,
            receipt_id,
            ..
        } = read_json_frame(&mut stream).await.unwrap()
        else {
            panic!("expected create");
        };
        write_json_frame(
            &mut stream,
            &AgentChatIntentFrame::Created {
                request_id,
                receipt: Receipt {
                    receipt_id,
                    idempotency_key: "redacted".into(),
                    status: ReceiptStatus::Settled,
                    host_epoch: HostEpoch(1),
                },
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: gent_types::AgentChatRunId("run-1".into()),
            },
        )
        .await
        .unwrap();
    });
    let reply = execute(
        Some(directory.path().into()),
        true,
        ChatCommand::Create(CreateArgs {
            provider: Provider::Claude,
            model: "haiku".into(),
            effort: Effort::Low,
            mode: Mode::Ask,
            request_id: Some("request-1".into()),
            receipt_id: Some("receipt-1".into()),
        }),
    )
    .await
    .unwrap();
    assert!(
        matches!(reply, AgentChatIntentFrame::Created { conversation_id, run_id, .. } if conversation_id.0 == "conversation-1" && run_id.0 == "run-1")
    );
}
