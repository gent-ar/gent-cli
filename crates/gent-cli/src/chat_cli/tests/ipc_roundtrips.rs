use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, Hello, Negotiated, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, CapabilitySet, HostEpoch, PROTOCOL_MAX, Receipt, ReceiptStatus,
};
use tokio::net::UnixListener;

use super::super::{ChatCommand, CreateArgs, Effort, Mode, Provider, execute};

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
            workspace: None,
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

#[tokio::test]
async fn switch_negotiates_a_parent_bound_child_run() {
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
        let AgentChatIntentFrame::SwitchSelection {
            request_id,
            receipt_id,
            conversation_id,
            parent_run_id,
            context_policy,
            ..
        } = read_json_frame(&mut stream).await.unwrap()
        else {
            panic!("expected switch");
        };
        write_json_frame(
            &mut stream,
            &AgentChatIntentFrame::Switched {
                request_id,
                receipt: Receipt {
                    receipt_id,
                    idempotency_key: "redacted".into(),
                    status: ReceiptStatus::Settled,
                    host_epoch: HostEpoch(1),
                },
                conversation_id,
                parent_run_id,
                run_id: gent_types::AgentChatRunId("run-2".into()),
                context_policy,
                context_through_ordinal: 1,
            },
        )
        .await
        .unwrap();
    });
    let reply = execute(
        Some(directory.path().into()),
        true,
        ChatCommand::Switch(super::super::switch::SwitchArgs {
            conversation_id: "conversation-1".into(),
            parent_run_id: Some("run-1".into()),
            provider: Provider::Codex,
            model: "gpt-5.6".into(),
            effort: Effort::High,
            mode: Mode::Agent,
            context: super::super::switch::Context::Preserve,
            request_id: Some("switch-1".into()),
            receipt_id: Some("receipt-1".into()),
        }),
    )
    .await
    .unwrap();
    assert!(
        matches!(reply, AgentChatIntentFrame::Switched { run_id, context_through_ordinal, .. } if run_id.0 == "run-2" && context_through_ordinal == 1)
    );
}

#[tokio::test]
async fn interrupt_negotiates_the_exact_durable_run() {
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
        let AgentChatIntentFrame::Interrupt {
            request_id,
            receipt_id,
            conversation_id,
            run_id,
        } = read_json_frame(&mut stream).await.unwrap()
        else {
            panic!("expected interrupt");
        };
        write_json_frame(
            &mut stream,
            &AgentChatIntentFrame::Interrupted {
                request_id,
                receipt: Receipt {
                    receipt_id,
                    idempotency_key: "redacted".into(),
                    status: ReceiptStatus::Settled,
                    host_epoch: HostEpoch(1),
                },
                conversation_id,
                run_id,
            },
        )
        .await
        .unwrap();
    });
    let reply = execute(
        Some(directory.path().into()),
        true,
        ChatCommand::Interrupt(super::super::interrupt::InterruptArgs {
            conversation: "conversation-1".into(),
            run: "run-1".into(),
            request: Some("interrupt-1".into()),
            receipt: Some("receipt-1".into()),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply,
        AgentChatIntentFrame::Interrupted { conversation_id, run_id, .. }
            if conversation_id.0 == "conversation-1" && run_id.0 == "run-1"
    ));
}
