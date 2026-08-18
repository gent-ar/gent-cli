use gent_protocol::{Negotiated, WireFrame, read_frame, write_frame};
use gent_types::{
    AgentChatRequestId, CapabilitySet, DurableTurnPhase, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, PROTOCOL_MAX, TurnTerminal,
};
use tokio::net::UnixListener;

use super::{FollowTurnArgs, accept_event, accept_terminal, run, supports};

#[test]
fn turn_follow_requires_its_dedicated_capability() {
    assert!(!supports(&[]));
    assert!(supports(&[
        gent_protocol::AGENT_CHAT_TURN_FOLLOW_CAPABILITY.into()
    ]));
}

#[test]
fn exact_request_and_monotonic_cursor_are_required() {
    let request = AgentChatRequestId("request-1".into());
    let mut cursor = 2;
    let event = event(3);
    assert!(accept_event(&request, "run", "turn", &mut cursor, &request, &event).is_ok());
    assert!(accept_event(&request, "run", "turn", &mut cursor, &request, &event).is_err());
    assert!(accept_event(&request, "other", "turn", &mut cursor, &request, &event).is_err());
    assert_eq!(cursor, 3);
}

#[test]
fn terminal_requires_the_exact_completed_turn_and_cursor() {
    let args = args();
    let request = AgentChatRequestId("request".into());
    let terminal = TurnTerminal {
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        turn_id: "turn".into(),
        phase: DurableTurnPhase::Completed,
        cursor: 4,
    };
    assert!(accept_terminal(&request, &args, 4, &request, &terminal).is_ok());
    assert!(accept_terminal(&request, &args, 3, &request, &terminal).is_err());
}

#[tokio::test]
async fn absent_capability_refuses_before_sending_a_follow_frame() {
    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("gentd.sock")).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame(&mut stream).await.unwrap(),
            WireFrame::Hello(_)
        ));
        write_frame(
            &mut stream,
            &WireFrame::Negotiated(Negotiated {
                protocol: PROTOCOL_MAX,
                capabilities: CapabilitySet::default(),
            }),
        )
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                read_frame(&mut stream)
            )
            .await
            .unwrap()
            .is_err()
        );
    });
    assert!(
        run(Some(directory.path().into()), true, args())
            .await
            .is_err()
    );
    server.await.unwrap();
}

fn args() -> FollowTurnArgs {
    FollowTurnArgs {
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        turn_id: "turn".into(),
        after_cursor: 0,
        reconnect_attempts: 3,
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
