use gent_protocol::{
    AgentChatTurnFollowEnd, AgentChatTurnFollowFrame, WireFrame, read_frame, read_json_frame,
    write_json_frame,
};
use gent_runtime::{TurnFollowRead, TurnFollowRequest};
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, DurableTurnPhase, HostEpoch,
    NormalizedTranscriptEvent, NormalizedTranscriptKind, TurnTerminal,
};
use tokio::io::duplex;

use super::{TurnFollowPort, send, serve};

#[test]
fn observer_and_persistence_only_catalogs_do_not_advertise_turn_follow() {
    for capabilities in [
        crate::transport::observed_capabilities(false, false, false, false),
        crate::transport::observed_capabilities(true, false, false, false),
    ] {
        assert!(
            !capabilities
                .0
                .iter()
                .any(|capability| capability == gent_protocol::AGENT_CHAT_TURN_FOLLOW_CAPABILITY)
        );
    }
}

#[derive(Clone)]
struct Source {
    read: TurnFollowRead,
    epoch: HostEpoch,
}

impl TurnFollowPort for Source {
    fn host_epoch(&self) -> Result<HostEpoch, String> {
        Ok(self.epoch)
    }

    fn read(&self, request: TurnFollowRequest) -> Result<TurnFollowRead, String> {
        (request.expected_host_epoch == self.epoch)
            .then_some(self.read.clone())
            .ok_or_else(|| "epoch changed".into())
    }
}

#[tokio::test]
async fn follows_exact_normalized_events_before_the_durable_terminal() {
    let (mut client, server) = duplex(16 * 1024);
    let task = tokio::spawn(serve(
        server,
        source(Some(terminal(2)), None),
        AgentChatRequestId("request-1".into()),
        AgentChatConversationId("conversation-1".into()),
        AgentChatRunId("run-1".into()),
        "turn-1".into(),
        0,
    ));
    assert!(matches!(
        read_json_frame::<_, AgentChatTurnFollowFrame>(&mut client).await.unwrap(),
        AgentChatTurnFollowFrame::Event { event, .. } if event.cursor == 2
    ));
    assert!(matches!(
        read_json_frame::<_, AgentChatTurnFollowFrame>(&mut client).await.unwrap(),
        AgentChatTurnFollowFrame::Terminal { terminal, .. }
            if terminal.is_valid() && terminal.cursor == 2
    ));
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn source_epoch_drift_requests_a_resync_without_provider_detail() {
    let (mut client, server) = duplex(16 * 1024);
    let source = Source {
        epoch: HostEpoch(2),
        ..source(None, None)
    };
    let task = tokio::spawn(serve(
        server,
        source,
        AgentChatRequestId("request-1".into()),
        AgentChatConversationId("conversation-1".into()),
        AgentChatRunId("run-1".into()),
        "turn-1".into(),
        0,
    ));
    assert!(matches!(
        read_json_frame::<_, AgentChatTurnFollowFrame>(&mut client)
            .await
            .unwrap(),
        AgentChatTurnFollowFrame::Ended {
            reason: AgentChatTurnFollowEnd::ResyncRequired,
            ..
        }
    ));
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn client_cannot_inject_a_turn_follow_event() {
    let (mut client, server) = duplex(16 * 1024);
    let task = tokio::spawn(serve(
        server,
        source(None, None),
        AgentChatRequestId("request-1".into()),
        AgentChatConversationId("conversation-1".into()),
        AgentChatRunId("run-1".into()),
        "turn-1".into(),
        0,
    ));
    write_json_frame(
        &mut client,
        &AgentChatTurnFollowFrame::Event {
            request_id: AgentChatRequestId("forged".into()),
            event: event(1),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Error { code, .. } if code == "invalidTurnFollowFrame"
    ));
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn emitted_cursor_is_a_valid_continuation_token() {
    let (mut reader, mut writer) = duplex(4096);
    let mut cursor = 0;
    assert!(
        !send(
            TurnFollowRead {
                host_epoch: HostEpoch(1),
                events: vec![event(1)],
                next_after_cursor: Some(1),
                terminal: None,
            },
            &mut cursor,
            &mut writer,
            &AgentChatRequestId("request-1".into()),
        )
        .await
        .unwrap()
    );
    assert_eq!(cursor, 1);
    assert!(matches!(
        read_json_frame::<_, AgentChatTurnFollowFrame>(&mut reader).await.unwrap(),
        AgentChatTurnFollowFrame::Event { event, .. } if event.cursor == 1
    ));
}

fn source(terminal: Option<TurnTerminal>, next_after_cursor: Option<u64>) -> Source {
    Source {
        epoch: HostEpoch(1),
        read: TurnFollowRead {
            host_epoch: HostEpoch(1),
            events: terminal
                .as_ref()
                .map(|_| vec![event(2)])
                .unwrap_or_default(),
            next_after_cursor,
            terminal,
        },
    }
}

fn terminal(cursor: u64) -> TurnTerminal {
    TurnTerminal {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        turn_id: "turn-1".into(),
        phase: DurableTurnPhase::Completed,
        cursor,
    }
}

fn event(cursor: u64) -> NormalizedTranscriptEvent {
    NormalizedTranscriptEvent {
        cursor,
        event_id: format!("event-{cursor}"),
        turn_id: "turn-1".into(),
        run_id: "run-1".into(),
        kind: NormalizedTranscriptKind::AssistantMessage,
        text: "normalized".into(),
        is_partial: false,
    }
}
