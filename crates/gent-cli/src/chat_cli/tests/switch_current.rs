use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_INTENTS_CAPABILITY, AgentChatConversationFrame,
    AgentChatIntentFrame, Negotiated, WireFrame, read_frame, read_json_frame, write_frame,
    write_json_frame,
};
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationSummary, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRun, AgentChatRunState, AgentChatSelection, CapabilitySet,
    HostEpoch, PROTOCOL_MAX, Receipt, ReceiptStatus,
};
use tokio::net::UnixListener;
use tokio::time::{Duration, timeout};

use crate::chat_cli::{ChatCommand, Effort, Mode, Provider, execute, switch};

#[tokio::test]
async fn switch_without_parent_resolves_the_durable_current_run_before_creating_a_child() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut detail_stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut detail_stream).await.unwrap();
        write_frame(
            &mut detail_stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![AGENT_CHAT_CONVERSATIONS_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            read_json_frame::<_, AgentChatConversationFrame>(&mut detail_stream)
                .await
                .unwrap(),
            AgentChatConversationFrame::DetailRequest {
                conversation_id: "conversation-1".into()
            }
        );
        write_json_frame(
            &mut detail_stream,
            &AgentChatConversationFrame::Detail(detail()),
        )
        .await
        .unwrap();
        let (mut switch_stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut switch_stream).await.unwrap();
        write_frame(
            &mut switch_stream,
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
        } = read_json_frame(&mut switch_stream).await.unwrap()
        else {
            panic!("expected switch")
        };
        assert_eq!(parent_run_id.0, "run-current");
        write_json_frame(
            &mut switch_stream,
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
                run_id: gent_types::AgentChatRunId("run-child".into()),
                context_policy,
                context_through_ordinal: 7,
            },
        )
        .await
        .unwrap();
    });
    let reply = execute(
        Some(directory.path().into()),
        true,
        ChatCommand::Switch(switch::SwitchArgs {
            conversation_id: "conversation-1".into(),
            parent_run_id: None,
            provider: Provider::Claurst,
            model: "qwen2-5-coder-7b-instruct-q4-k-m".into(),
            effort: Effort::Medium,
            mode: Mode::Agent,
            context: switch::Context::Preserve,
            request_id: Some("switch-current".into()),
            receipt_id: Some("receipt-current".into()),
        }),
    )
    .await
    .unwrap();
    assert!(
        matches!(reply, AgentChatIntentFrame::Switched { run_id, .. } if run_id.0 == "run-child")
    );
}

#[tokio::test]
async fn switch_without_parent_refuses_an_unknown_durable_current_run_before_emitting_an_intent() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![AGENT_CHAT_CONVERSATIONS_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        assert!(matches!(
            read_json_frame::<_, AgentChatConversationFrame>(&mut stream)
                .await
                .unwrap(),
            AgentChatConversationFrame::DetailRequest { conversation_id }
                if conversation_id == "conversation-1"
        ));
        let mut value = detail();
        value.current_run_id = "missing-run".into();
        write_json_frame(&mut stream, &AgentChatConversationFrame::Detail(value))
            .await
            .unwrap();
        assert!(
            timeout(Duration::from_millis(150), listener.accept())
                .await
                .is_err()
        );
    });
    let error = execute(
        Some(directory.path().into()),
        true,
        ChatCommand::Switch(switch::SwitchArgs {
            conversation_id: "conversation-1".into(),
            parent_run_id: None,
            provider: Provider::Claude,
            model: "sonnet".into(),
            effort: Effort::Medium,
            mode: Mode::Agent,
            context: switch::Context::Preserve,
            request_id: Some("switch-invalid-current".into()),
            receipt_id: Some("receipt-invalid-current".into()),
        }),
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid current run for this conversation")
    );
    server.await.unwrap();
}

fn detail() -> AgentChatConversationDetail {
    let selection = AgentChatSelection {
        provider: AgentChatProvider::Codex,
        model: "gpt-5.6".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
    };
    AgentChatConversationDetail {
        summary: AgentChatConversationSummary {
            conversation_id: "conversation-1".into(),
            title: None,
            recap: None,
            workspace_id: None,
            workspace_path: None,
            mcp_server_count: 0,
            mcp_server_names: Vec::new(),
            changed_file_count: None,
            git_branch: None,
            updated_at_unix_ms: 1,
            selection: selection.clone(),
        },
        current_run_id: "run-current".into(),
        runs: vec![AgentChatRun {
            run_id: "run-current".into(),
            parent_run_id: None,
            selection,
            state: AgentChatRunState::Completed,
        }],
    }
}
