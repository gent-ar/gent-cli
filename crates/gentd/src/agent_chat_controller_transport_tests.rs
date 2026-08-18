use gent_protocol::{
    AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY, AgentChatControllerDelta, AgentChatControllerSnapshot,
    AgentChatControllerStreamEnd, AgentChatControllerStreamFrame, WireFrame, read_frame,
    write_json_frame,
};
use gent_runtime::AgentChatControllerDeltaPage;
use gent_types::{
    AgentChatConversationDetail, AgentChatConversationSummary, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatSelection, ConversationStatus, HostEpoch,
    NormalizedTranscriptEvent, NormalizedTranscriptKind, NormalizedTranscriptPage,
};
use tokio::io::duplex;

use super::serve;
use crate::transport::observed_capabilities;

#[derive(Clone)]
struct Source;

#[derive(Clone)]
struct Unavailable;

#[derive(Clone)]
struct DeltaSource;

impl super::ControllerStreamPort for Source {
    fn snapshot(
        &self,
        conversation_id: &str,
        after_cursor: u64,
    ) -> Result<AgentChatControllerSnapshot, String> {
        Ok(snapshot(conversation_id, after_cursor))
    }

    fn delta(
        &self,
        _: &str,
        _: u64,
        host_epoch: HostEpoch,
    ) -> Result<AgentChatControllerDeltaPage, String> {
        Ok(AgentChatControllerDeltaPage {
            host_epoch,
            events: Vec::new(),
        })
    }
}

impl super::ControllerStreamPort for Unavailable {
    fn snapshot(&self, _: &str, _: u64) -> Result<AgentChatControllerSnapshot, String> {
        Err("observer-disabled".into())
    }

    fn delta(&self, _: &str, _: u64, _: HostEpoch) -> Result<AgentChatControllerDeltaPage, String> {
        Err("observer-disabled".into())
    }
}

impl super::ControllerStreamPort for DeltaSource {
    fn snapshot(
        &self,
        conversation_id: &str,
        after_cursor: u64,
    ) -> Result<AgentChatControllerSnapshot, String> {
        Ok(snapshot(conversation_id, after_cursor))
    }

    fn delta(
        &self,
        _: &str,
        after_cursor: u64,
        host_epoch: HostEpoch,
    ) -> Result<AgentChatControllerDeltaPage, String> {
        Ok(AgentChatControllerDeltaPage {
            host_epoch,
            events: (after_cursor < 2).then(|| event(2)).into_iter().collect(),
        })
    }
}

#[test]
fn observer_never_advertises_the_unwired_controller_stream() {
    assert!(
        !observed_capabilities(false, false, false)
            .0
            .iter()
            .any(|item| item == AGENT_CHAT_CONTROLLER_STREAM_CAPABILITY)
    );
}

#[tokio::test]
async fn attach_receives_snapshot_and_accepts_a_bounded_ack() {
    let (client, server) = duplex(4096);
    let task = tokio::spawn(serve(server, Source));
    let (mut reader, mut writer) = tokio::io::split(client);
    attach(&mut writer).await;
    assert!(matches!(
        gent_protocol::read_json_frame::<_, AgentChatControllerStreamFrame>(&mut reader)
            .await
            .unwrap(),
        AgentChatControllerStreamFrame::Snapshot(snapshot) if snapshot.cursor == 1
    ));
    write_json_frame(
        &mut writer,
        &AgentChatControllerStreamFrame::Ack { cursor: 1 },
    )
    .await
    .unwrap();
    drop(writer);
    drop(reader);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn durable_delta_waits_for_the_snapshot_ack_then_requires_its_own_ack() {
    let (client, server) = duplex(4096);
    let task = tokio::spawn(serve(server, DeltaSource));
    let (mut reader, mut writer) = tokio::io::split(client);
    attach(&mut writer).await;
    let snapshot = gent_protocol::read_json_frame::<_, AgentChatControllerStreamFrame>(&mut reader)
        .await
        .unwrap();
    assert!(
        matches!(snapshot, AgentChatControllerStreamFrame::Snapshot(value) if value.cursor == 1)
    );
    write_json_frame(
        &mut writer,
        &AgentChatControllerStreamFrame::Ack { cursor: 1 },
    )
    .await
    .unwrap();
    assert!(matches!(
        tokio::time::timeout(
            std::time::Duration::from_millis(250),
            gent_protocol::read_json_frame::<_, AgentChatControllerStreamFrame>(&mut reader)
        )
        .await
        .unwrap()
        .unwrap(),
        AgentChatControllerStreamFrame::Delta(AgentChatControllerDelta::Transcript { event, .. })
            if event.cursor == 2
    ));
    write_json_frame(
        &mut writer,
        &AgentChatControllerStreamFrame::Ack { cursor: 2 },
    )
    .await
    .unwrap();
    drop(writer);
    drop(reader);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn out_of_range_ack_ends_the_stream_without_accepting_client_state() {
    let (client, server) = duplex(4096);
    let task = tokio::spawn(serve(server, Source));
    let (mut reader, mut writer) = tokio::io::split(client);
    attach(&mut writer).await;
    let _ = gent_protocol::read_json_frame::<_, AgentChatControllerStreamFrame>(&mut reader)
        .await
        .unwrap();
    write_json_frame(
        &mut writer,
        &AgentChatControllerStreamFrame::Ack { cursor: 2 },
    )
    .await
    .unwrap();
    assert!(
        matches!(read_frame(&mut reader).await.unwrap(), WireFrame::Error { code, .. } if code == "invalidAgentChatControllerAck")
    );
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn unavailable_snapshot_returns_a_server_end_frame() {
    let (client, server) = duplex(4096);
    let task = tokio::spawn(serve(server, Unavailable));
    let (mut reader, mut writer) = tokio::io::split(client);
    attach(&mut writer).await;
    assert!(matches!(
        gent_protocol::read_json_frame::<_, AgentChatControllerStreamFrame>(&mut reader)
            .await
            .unwrap(),
        AgentChatControllerStreamFrame::End {
            reason: AgentChatControllerStreamEnd::ConversationUnavailable
        }
    ));
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn server_rejects_client_snapshot_before_it_can_be_accepted() {
    let (client, server) = duplex(4096);
    let task = tokio::spawn(serve(server, Source));
    let (mut reader, mut writer) = tokio::io::split(client);
    write_json_frame(
        &mut writer,
        &AgentChatControllerStreamFrame::Snapshot(snapshot("c", 0)),
    )
    .await
    .unwrap();
    assert!(
        matches!(read_frame(&mut reader).await.unwrap(), WireFrame::Error { code, .. } if code == "invalidAgentChatControllerFrame")
    );
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn server_rejects_client_delta_after_a_server_snapshot() {
    let (client, server) = duplex(4096);
    let task = tokio::spawn(serve(server, Source));
    let (mut reader, mut writer) = tokio::io::split(client);
    attach(&mut writer).await;
    assert!(matches!(
        gent_protocol::read_json_frame::<_, AgentChatControllerStreamFrame>(&mut reader)
            .await
            .unwrap(),
        AgentChatControllerStreamFrame::Snapshot(_)
    ));
    write_json_frame(
        &mut writer,
        &AgentChatControllerStreamFrame::Delta(AgentChatControllerDelta::Transcript {
            host_epoch: HostEpoch(1),
            event: event(1),
        }),
    )
    .await
    .unwrap();
    assert!(
        matches!(read_frame(&mut reader).await.unwrap(), WireFrame::Error { code, .. } if code == "invalidAgentChatControllerFrame")
    );
    task.await.unwrap().unwrap();
}

async fn attach<W>(writer: &mut W)
where
    W: tokio::io::AsyncWrite + Unpin,
{
    write_json_frame(
        writer,
        &AgentChatControllerStreamFrame::Attach {
            conversation_id: "c".into(),
            after_cursor: 0,
        },
    )
    .await
    .unwrap();
}

fn snapshot(conversation_id: &str, after_cursor: u64) -> AgentChatControllerSnapshot {
    AgentChatControllerSnapshot {
        host_epoch: HostEpoch(1),
        conversation: AgentChatConversationDetail {
            summary: AgentChatConversationSummary {
                conversation_id: conversation_id.into(),
                title: None,
                updated_at_unix_ms: 1,
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "model".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Ask,
                },
            },
            runs: Vec::new(),
        },
        transcript: NormalizedTranscriptPage {
            conversation_id: conversation_id.into(),
            events: vec![event(after_cursor + 1)],
            next_after_cursor: None,
        },
        cursor: after_cursor + 1,
        status: Some(ConversationStatus {
            conversation_id: conversation_id.into(),
            runs: Vec::new(),
        }),
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
