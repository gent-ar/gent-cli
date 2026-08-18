use gent_protocol::{
    AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY, AgentChatControllerDelta, AgentChatControllerSnapshot,
    AgentChatControllerStreamEnd, AgentChatControllerStreamFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationSummary, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatSelection, HostEpoch, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, NormalizedTranscriptPage,
};
use tokio::io::duplex;

use super::{
    ControllerStream, ControllerStreamError, ControllerStreamEvent, supports_controller_stream,
};

#[tokio::test]
async fn applies_snapshot_delta_and_resync_before_acknowledging_each_cursor() {
    let (client, mut daemon) = duplex(4096);
    let server = tokio::spawn(async move {
        assert!(matches!(
            read_json_frame::<_, AgentChatControllerStreamFrame>(&mut daemon)
                .await
                .unwrap(),
            AgentChatControllerStreamFrame::Attach { conversation_id, after_cursor }
                if conversation_id == "conversation" && after_cursor == 0
        ));
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Snapshot(snapshot(1)),
        )
        .await
        .unwrap();
        assert_ack(&mut daemon, 1).await;
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Delta(AgentChatControllerDelta::Transcript {
                host_epoch: HostEpoch(7),
                event: event(2),
            }),
        )
        .await
        .unwrap();
        assert_ack(&mut daemon, 2).await;
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Resync(snapshot(5)),
        )
        .await
        .unwrap();
        assert_ack(&mut daemon, 5).await;
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::End {
                reason: AgentChatControllerStreamEnd::ServerClosing,
            },
        )
        .await
        .unwrap();
    });
    let mut stream = ControllerStream::attach(
        client,
        &[AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY.into()],
        "conversation".into(),
        0,
    )
    .await
    .unwrap();
    assert_eq!(
        stream.receive().await.unwrap(),
        ControllerStreamEvent::ProjectionReplaced
    );
    assert_eq!(
        stream.receive().await.unwrap(),
        ControllerStreamEvent::TranscriptApplied
    );
    assert_eq!(stream.projection().unwrap().transcript().cursor(), 2);
    assert_eq!(
        stream.receive().await.unwrap(),
        ControllerStreamEvent::ProjectionReplaced
    );
    assert_eq!(stream.projection().unwrap().transcript().cursor(), 5);
    assert_eq!(
        stream.receive().await.unwrap(),
        ControllerStreamEvent::ReconnectRequired
    );
    server.await.unwrap();
}

#[tokio::test]
async fn rejects_unnegotiated_stream_and_delta_before_snapshot() {
    let (client, _) = duplex(256);
    assert!(matches!(
        ControllerStream::attach(client, &[], "conversation".into(), 0).await,
        Err(ControllerStreamError::UnsupportedCapability)
    ));
    assert!(!supports_controller_stream(&[]));

    let (client, mut daemon) = duplex(1024);
    let server = tokio::spawn(async move {
        assert!(matches!(
            read_json_frame::<_, AgentChatControllerStreamFrame>(&mut daemon)
                .await
                .unwrap(),
            AgentChatControllerStreamFrame::Attach { .. }
        ));
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Delta(AgentChatControllerDelta::Transcript {
                host_epoch: HostEpoch(7),
                event: event(1),
            }),
        )
        .await
        .unwrap();
    });
    let mut stream = ControllerStream::attach(
        client,
        &[AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY.into()],
        "conversation".into(),
        0,
    )
    .await
    .unwrap();
    assert!(matches!(
        stream.receive().await,
        Err(ControllerStreamError::MissingSnapshot)
    ));
    server.await.unwrap();
}

async fn assert_ack<S: tokio::io::AsyncRead + Unpin>(socket: &mut S, cursor: u64) {
    assert!(
        matches!(read_json_frame::<_, AgentChatControllerStreamFrame>(socket).await.unwrap(), AgentChatControllerStreamFrame::Ack { cursor: actual } if actual == cursor)
    );
}

fn snapshot(cursor: u64) -> AgentChatControllerSnapshot {
    AgentChatControllerSnapshot {
        host_epoch: HostEpoch(7),
        conversation: AgentChatConversationDetail {
            summary: AgentChatConversationSummary {
                conversation_id: "conversation".into(),
                title: None,
                updated_at_unix_ms: 1,
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt".into(),
                    effort: AgentChatEffort::Low,
                    mode: AgentChatMode::Ask,
                },
            },
            runs: vec![],
        },
        transcript: NormalizedTranscriptPage {
            conversation_id: "conversation".into(),
            events: vec![event(cursor)],
            next_after_cursor: None,
        },
        cursor,
        status: None,
    }
}

fn event(cursor: u64) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("event-{cursor}"),
        turn_id: "turn".into(),
        run_id: "run".into(),
        kind: NormalizedTranscriptKind::AssistantMessage,
        text: "normalized".into(),
        is_partial: false,
    }
}
