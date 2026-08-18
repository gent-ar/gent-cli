use gent_protocol::{
    AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY, AgentChatControllerDelta, AgentChatControllerSnapshot,
    AgentChatControllerStreamFrame, read_json_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationSummary, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatSelection, HostEpoch, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, NormalizedTranscriptPage,
};
use tokio::io::duplex;

use super::{ControllerStream, ControllerStreamError, ControllerStreamEvent};

#[tokio::test]
async fn resume_retains_the_requested_cursor_until_a_projection_arrives() {
    let (client, mut daemon) = duplex(256);
    let server = tokio::spawn(async move {
        read_json_frame::<_, AgentChatControllerStreamFrame>(&mut daemon)
            .await
            .unwrap();
    });
    let stream = attach(client, 9).await;
    assert_eq!(stream.resume().conversation_id(), "conversation");
    assert_eq!(stream.resume().after_cursor(), 9);
    server.await.unwrap();
}

#[tokio::test]
async fn reconnect_cursor_follows_applied_state_even_when_acknowledgement_cannot_be_written() {
    let (client, mut daemon) = duplex(1024);
    let server = tokio::spawn(async move {
        assert!(matches!(
            read_json_frame::<_, AgentChatControllerStreamFrame>(&mut daemon)
                .await
                .unwrap(),
            AgentChatControllerStreamFrame::Attach {
                after_cursor: 4,
                ..
            }
        ));
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Snapshot(snapshot(5)),
        )
        .await
        .unwrap();
        assert_ack(&mut daemon, 5).await;
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Delta(AgentChatControllerDelta::Transcript {
                host_epoch: HostEpoch(7),
                event: event(6),
            }),
        )
        .await
        .unwrap();
    });
    let mut stream = attach(client, 4).await;
    assert_eq!(
        stream.receive().await.unwrap(),
        ControllerStreamEvent::ProjectionReplaced
    );
    server.await.unwrap();
    assert!(matches!(
        stream.receive().await,
        Err(ControllerStreamError::Io(_))
    ));
    assert_eq!(stream.resume().conversation_id(), "conversation");
    assert_eq!(stream.resume().after_cursor(), 6);
}

#[tokio::test]
async fn resumes_exactly_from_last_projection_and_rejects_snapshot_regressions() {
    let (client, mut daemon) = duplex(1024);
    let server = tokio::spawn(async move {
        assert!(matches!(
            read_json_frame::<_, AgentChatControllerStreamFrame>(&mut daemon)
                .await
                .unwrap(),
            AgentChatControllerStreamFrame::Attach {
                after_cursor: 4,
                ..
            }
        ));
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Snapshot(snapshot(5)),
        )
        .await
        .unwrap();
        assert_ack(&mut daemon, 5).await;
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Resync(snapshot(4)),
        )
        .await
        .unwrap();
    });
    let mut stream = attach(client, 4).await;
    assert_eq!(
        stream.receive().await.unwrap(),
        ControllerStreamEvent::ProjectionReplaced
    );
    assert_eq!(stream.resume().after_cursor(), 5);
    assert!(matches!(
        stream.receive().await,
        Err(ControllerStreamError::InvalidSnapshot)
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn rejects_resync_before_snapshot_and_a_second_snapshot() {
    let (client, mut daemon) = duplex(1024);
    let server = tokio::spawn(async move {
        read_json_frame::<_, AgentChatControllerStreamFrame>(&mut daemon)
            .await
            .unwrap();
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Resync(snapshot(1)),
        )
        .await
        .unwrap();
    });
    let mut stream = attach(client, 0).await;
    assert!(matches!(
        stream.receive().await,
        Err(ControllerStreamError::MissingSnapshot)
    ));
    server.await.unwrap();

    let (client, mut daemon) = duplex(1024);
    let server = tokio::spawn(async move {
        read_json_frame::<_, AgentChatControllerStreamFrame>(&mut daemon)
            .await
            .unwrap();
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Snapshot(snapshot(1)),
        )
        .await
        .unwrap();
        assert_ack(&mut daemon, 1).await;
        write_json_frame(
            &mut daemon,
            &AgentChatControllerStreamFrame::Snapshot(snapshot(2)),
        )
        .await
        .unwrap();
    });
    let mut stream = attach(client, 0).await;
    stream.receive().await.unwrap();
    assert!(matches!(
        stream.receive().await,
        Err(ControllerStreamError::DuplicateSnapshot)
    ));
    server.await.unwrap();
}

async fn attach(
    socket: tokio::io::DuplexStream,
    after_cursor: u64,
) -> ControllerStream<tokio::io::DuplexStream> {
    ControllerStream::attach(
        socket,
        &[AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY.into()],
        "conversation".into(),
        after_cursor,
    )
    .await
    .unwrap()
}

async fn assert_ack<S: tokio::io::AsyncRead + Unpin>(socket: &mut S, cursor: u64) {
    assert!(matches!(
        read_json_frame::<_, AgentChatControllerStreamFrame>(socket).await.unwrap(),
        AgentChatControllerStreamFrame::Ack { cursor: actual } if actual == cursor
    ));
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
