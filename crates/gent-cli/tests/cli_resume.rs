#![cfg(unix)]

use std::process::Command;

use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_TURN_FOLLOW_CAPABILITY, AgentChatIntentFrame,
    AgentChatTurnFollowFrame, Hello, Negotiated, WireFrame, read_frame, read_json_frame,
    write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatPromptDelivery, AgentChatRunId, CapabilitySet,
    DurableTurnPhase, HostEpoch, PROTOCOL_MAX, Receipt, ReceiptStatus, TurnTerminal,
};
use tokio::net::{UnixListener, UnixStream};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_resume_submits_to_the_existing_gent_conversation_without_native_session_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        negotiate(&mut stream, AGENT_CHAT_INTENTS_CAPABILITY).await;
        let AgentChatIntentFrame::SendPrompt {
            request_id,
            receipt_id,
            conversation_id,
            text,
            attachment_ids,
        } = read_json_frame(&mut stream).await.unwrap()
        else {
            panic!("resume must send a provider-neutral prompt")
        };
        assert_eq!(conversation_id.0, "conversation-1");
        assert_eq!(text, "continue after the first Codex turn");
        assert!(attachment_ids.is_empty());
        write_json_frame(
            &mut stream,
            &AgentChatIntentFrame::Accepted {
                request_id,
                receipt: Receipt {
                    receipt_id,
                    idempotency_key: "resume".into(),
                    status: ReceiptStatus::Accepted,
                    host_epoch: HostEpoch(1),
                },
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
                turn_id: "turn-2".into(),
                delivery: AgentChatPromptDelivery::AwaitingProvider,
            },
        )
        .await
        .unwrap();
        let _ = accept_and_negotiate(&listener, AGENT_CHAT_TURN_FOLLOW_CAPABILITY).await;
        let _ = accept_and_negotiate(&listener, AGENT_CHAT_TURN_FOLLOW_CAPABILITY).await;
        let mut follow_stream =
            accept_and_negotiate(&listener, AGENT_CHAT_TURN_FOLLOW_CAPABILITY).await;
        let AgentChatTurnFollowFrame::Follow {
            request_id,
            conversation_id,
            run_id,
            turn_id,
            ..
        } = read_json_frame(&mut follow_stream).await.unwrap()
        else {
            panic!("resume must follow its accepted durable turn")
        };
        assert_eq!(conversation_id.0, "conversation-1");
        assert_eq!(run_id.0, "run-1");
        assert_eq!(turn_id, "turn-2");
        write_json_frame(
            &mut follow_stream,
            &AgentChatTurnFollowFrame::Terminal {
                request_id,
                terminal: TurnTerminal {
                    conversation_id: "conversation-1".into(),
                    run_id: "run-1".into(),
                    turn_id: "turn-2".into(),
                    phase: DurableTurnPhase::Completed,
                    cursor: 0,
                },
            },
        )
        .await
        .unwrap();
    });
    let output = Command::new(env!("CARGO_BIN_EXE_gent"))
        .args([
            "--data-dir",
            directory.path().to_str().unwrap(),
            "--no-autostart",
            "chat",
            "resume",
            "conversation-1",
            "continue after the first Codex turn",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("conversation-1"));
    assert!(stdout.contains("turn-2"));
    assert!(stdout.contains("completed"));
}

async fn accept_and_negotiate(listener: &UnixListener, capability: &str) -> UnixStream {
    let (mut stream, _) = listener.accept().await.unwrap();
    negotiate(&mut stream, capability).await;
    stream
}

async fn negotiate(stream: &mut UnixStream, capability: &str) {
    assert!(matches!(
        read_frame(stream).await.unwrap(),
        WireFrame::Hello(Hello { capabilities, .. })
            if capabilities.0.contains(&capability.to_owned())
    ));
    write_frame(
        stream,
        &WireFrame::Negotiated(Negotiated {
            protocol: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![capability.into()]),
        }),
    )
    .await
    .unwrap();
}
