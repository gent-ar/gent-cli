use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, Hello, Negotiated, WireFrame, read_frame,
    read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, CapabilitySet, HostEpoch, PROTOCOL_MAX, Receipt, ReceiptStatus,
};
use tokio::net::UnixListener;

use super::{ChatCommand, CreateArgs, Effort, Mode, Provider, execute, frame};

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
        ChatCommand::Switch(super::switch::SwitchArgs {
            conversation_id: "conversation-1".into(),
            parent_run_id: "run-1".into(),
            provider: Provider::Codex,
            model: "gpt-5.6".into(),
            effort: Effort::High,
            mode: Mode::Agent,
            context: super::switch::Context::Preserve,
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

#[test]
fn selection_switch_carries_each_provider_model_effort_mode_and_context_policy() {
    let cases = [
        (
            Provider::Codex,
            Effort::Low,
            Mode::Ask,
            super::switch::Context::Preserve,
            gent_types::AgentChatProvider::Codex,
            gent_types::AgentChatEffort::Low,
            gent_types::AgentChatMode::Ask,
            gent_types::ContextPolicy::Preserve,
        ),
        (
            Provider::Claude,
            Effort::Medium,
            Mode::Plan,
            super::switch::Context::Clear,
            gent_types::AgentChatProvider::Claude,
            gent_types::AgentChatEffort::Medium,
            gent_types::AgentChatMode::Plan,
            gent_types::ContextPolicy::Clear,
        ),
        (
            Provider::Claurst,
            Effort::High,
            Mode::Agent,
            super::switch::Context::Preserve,
            gent_types::AgentChatProvider::Claurst,
            gent_types::AgentChatEffort::High,
            gent_types::AgentChatMode::Agent,
            gent_types::ContextPolicy::Preserve,
        ),
    ];
    for (
        provider,
        effort,
        mode,
        context,
        expected_provider,
        expected_effort,
        expected_mode,
        expected_context,
    ) in cases
    {
        let request = frame(ChatCommand::Switch(super::switch::SwitchArgs {
            conversation_id: "conversation-1".into(),
            parent_run_id: "run-1".into(),
            provider,
            model: "exact-model".into(),
            effort,
            mode,
            context,
            request_id: Some("request-1".into()),
            receipt_id: Some("receipt-1".into()),
        }));
        let AgentChatIntentFrame::SwitchSelection {
            selection,
            context_policy,
            ..
        } = request
        else {
            panic!("expected selection switch");
        };
        assert_eq!(selection.provider, expected_provider);
        assert_eq!(selection.model, "exact-model");
        assert_eq!(selection.effort, expected_effort);
        assert_eq!(selection.mode, expected_mode);
        assert_eq!(context_policy, expected_context);
    }
}
