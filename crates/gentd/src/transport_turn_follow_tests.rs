use gent_protocol::{
    AGENT_CHAT_TURN_FOLLOW_CAPABILITY, AgentChatTurnFollowFrame, Hello, Negotiated, WireFrame,
    read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_runtime::TurnFollowRead;
use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, CapabilitySet, DurableTurnPhase,
    HostEpoch, NormalizedTranscriptEvent, NormalizedTranscriptKind, PROTOCOL_MAX, PROTOCOL_MIN,
    TurnTerminal,
};
use tokio::io::duplex;

use crate::transport::serve_connection;
use crate::transport_tests::FakeRuntime;

pub(crate) fn read() -> TurnFollowRead {
    TurnFollowRead {
        host_epoch: HostEpoch(1),
        events: vec![NormalizedTranscriptEvent {
            cursor: 1,
            event_id: "event-1".into(),
            turn_id: "turn-1".into(),
            run_id: "run-1".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: "normalized".into(),
            is_partial: false,
        }],
        next_after_cursor: None,
        terminal: Some(TurnTerminal {
            conversation_id: "conversation-1".into(),
            run_id: "run-1".into(),
            turn_id: "turn-1".into(),
            phase: DurableTurnPhase::Completed,
            cursor: 1,
        }),
    }
}

#[tokio::test]
async fn negotiated_turn_follow_routes_only_to_the_read_only_turn_source() {
    let (mut client, server) = duplex(16 * 1024);
    let task = tokio::spawn(serve_connection(server, FakeRuntime));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![AGENT_CHAT_TURN_FOLLOW_CAPABILITY.into()]),
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        read_frame(&mut client).await.unwrap(),
        WireFrame::Negotiated(Negotiated { capabilities, .. })
            if capabilities.0 == vec![AGENT_CHAT_TURN_FOLLOW_CAPABILITY]
    ));
    write_json_frame(
        &mut client,
        &AgentChatTurnFollowFrame::Follow {
            request_id: AgentChatRequestId("request-1".into()),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
            turn_id: "turn-1".into(),
            after_cursor: 0,
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_json_frame::<_, AgentChatTurnFollowFrame>(&mut client).await.unwrap(),
        AgentChatTurnFollowFrame::Event { event, .. } if event.cursor == 1
    ));
    assert!(matches!(
        read_json_frame::<_, AgentChatTurnFollowFrame>(&mut client).await.unwrap(),
        AgentChatTurnFollowFrame::Terminal { terminal, .. } if terminal.is_valid()
    ));
    task.await.unwrap().unwrap();
}
