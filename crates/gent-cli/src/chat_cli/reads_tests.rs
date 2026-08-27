use gent_protocol::{Hello, Negotiated, read_frame, write_frame};
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection, CapabilitySet,
    NormalizedTranscriptEvent, NormalizedTranscriptKind, PROTOCOL_MAX,
};
use tokio::net::UnixListener;

use super::*;

#[tokio::test]
async fn transcript_rejects_a_zero_limit_before_connecting() {
    let directory = tempfile::tempdir().unwrap();
    let error = transcript(
        Some(directory.path().into()),
        true,
        "conversation-1".into(),
        None,
        0,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("at least 1"));
}

#[tokio::test]
async fn summary_sends_only_a_typed_read_after_negotiation() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(
            matches!(read_frame(&mut stream).await.unwrap(), WireFrame::Hello(Hello { capabilities, .. }) if capabilities.0.contains(&AGENT_CHAT_CONVERSATIONS_CAPABILITY.into()))
        );
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![AGENT_CHAT_CONVERSATIONS_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            read_json_frame::<_, AgentChatConversationFrame>(&mut stream)
                .await
                .unwrap(),
            AgentChatConversationFrame::SummaryRequest {
                conversation_id: "c1".into()
            }
        );
        write_json_frame(
            &mut stream,
            &AgentChatConversationFrame::Summary(test_summary("c1")),
        )
        .await
        .unwrap();
    });
    assert_eq!(
        summary(Some(directory.path().into()), true, "c1".into())
            .await
            .unwrap()
            .conversation_id,
        "c1"
    );
}

#[tokio::test]
async fn transcript_requires_its_capability_without_sending_a_request() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet::default(),
            }),
        )
        .await
        .unwrap();
    });
    assert!(
        transcript(Some(directory.path().into()), true, "c1".into(), None, 20)
            .await
            .unwrap_err()
            .to_string()
            .contains("upgrade gentd")
    );
}

#[tokio::test]
async fn transcript_rejects_a_nonascending_daemon_page() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![AGENT_CHAT_TRANSCRIPT_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        let _ = read_json_frame::<_, AgentChatTranscriptFrame>(&mut stream)
            .await
            .unwrap();
        write_json_frame(
            &mut stream,
            &AgentChatTranscriptFrame::Page(NormalizedTranscriptPage {
                conversation_id: "c1".into(),
                events: vec![event(2), event(2)],
                next_after_cursor: None,
            }),
        )
        .await
        .unwrap();
    });
    assert!(
        transcript(
            Some(directory.path().into()),
            true,
            "c1".into(),
            Some(1),
            20
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn transcript_rejects_an_event_without_shared_durable_ids() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet(vec![AGENT_CHAT_TRANSCRIPT_CAPABILITY.into()]),
            }),
        )
        .await
        .unwrap();
        let _ = read_json_frame::<_, AgentChatTranscriptFrame>(&mut stream)
            .await
            .unwrap();
        let mut invalid = event(1);
        invalid.turn_id.clear();
        write_json_frame(
            &mut stream,
            &AgentChatTranscriptFrame::Page(NormalizedTranscriptPage {
                conversation_id: "c1".into(),
                events: vec![invalid],
                next_after_cursor: None,
            }),
        )
        .await
        .unwrap();
    });
    assert!(
        transcript(Some(directory.path().into()), true, "c1".into(), None, 20)
            .await
            .is_err()
    );
}

fn test_summary(conversation_id: &str) -> AgentChatConversationSummary {
    AgentChatConversationSummary {
        conversation_id: conversation_id.into(),
        title: None,
        recap: None,
        workspace_id: None,
        workspace_path: None,
        mcp_server_count: 0,
        mcp_server_names: Vec::new(),
        changed_file_count: None,
        git_branch: None,
        updated_at_unix_ms: 1,
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt".into(),
            effort: AgentChatEffort::Low,
            mode: AgentChatMode::Ask,
        },
    }
}

fn event(cursor: u64) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("e{cursor}"),
        turn_id: "t1".into(),
        run_id: "r1".into(),
        kind: NormalizedTranscriptKind::AssistantMessage,
        text: "ok".into(),
        is_partial: false,
    }
}
