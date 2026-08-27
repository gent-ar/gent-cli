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
use tokio::net::UnixListener;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn direct_prompt_follows_a_negotiated_live_turn() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(serve(listener));
    let output = Command::new(env!("CARGO_BIN_EXE_gent"))
        .args([
            "--data-dir",
            directory.path().to_str().unwrap(),
            "--no-autostart",
            "reply briefly",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("conversation-1") && stdout.contains("completed"));
}

async fn serve(listener: UnixListener) {
    for index in 0..5 {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(Hello { .. })
        ));
        let capabilities = if index < 2 {
            CapabilitySet(vec![AGENT_CHAT_INTENTS_CAPABILITY.into()])
        } else {
            CapabilitySet(vec![AGENT_CHAT_TURN_FOLLOW_CAPABILITY.into()])
        };
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities,
            }),
        )
        .await
        .unwrap();
        if index == 2 || index == 3 {
            continue;
        }
        if index == 0 {
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
                    receipt: receipt(receipt_id, "create"),
                    conversation_id: AgentChatConversationId("conversation-1".into()),
                    run_id: AgentChatRunId("run-1".into()),
                },
            )
            .await
            .unwrap();
        } else if index == 1 {
            let AgentChatIntentFrame::SendPrompt {
                request_id,
                receipt_id,
                attachment_ids,
                ..
            } = read_json_frame(&mut stream).await.unwrap()
            else {
                panic!("expected prompt");
            };
            assert!(attachment_ids.is_empty());
            write_json_frame(
                &mut stream,
                &AgentChatIntentFrame::Accepted {
                    request_id,
                    receipt: receipt(receipt_id, "send"),
                    conversation_id: AgentChatConversationId("conversation-1".into()),
                    run_id: AgentChatRunId("run-1".into()),
                    turn_id: "turn-1".into(),
                    delivery: AgentChatPromptDelivery::AwaitingProvider,
                },
            )
            .await
            .unwrap();
        } else {
            let AgentChatTurnFollowFrame::Follow { request_id, .. } =
                read_json_frame(&mut stream).await.unwrap()
            else {
                panic!("expected turn follow");
            };
            write_json_frame(
                &mut stream,
                &AgentChatTurnFollowFrame::Terminal {
                    request_id,
                    terminal: TurnTerminal {
                        conversation_id: "conversation-1".into(),
                        run_id: "run-1".into(),
                        turn_id: "turn-1".into(),
                        phase: DurableTurnPhase::Completed,
                        cursor: 0,
                    },
                },
            )
            .await
            .unwrap();
        }
    }
}

fn receipt(receipt_id: gent_types::ReceiptId, key: &str) -> Receipt {
    Receipt {
        receipt_id,
        idempotency_key: key.into(),
        status: ReceiptStatus::Settled,
        host_epoch: HostEpoch(7),
    }
}
