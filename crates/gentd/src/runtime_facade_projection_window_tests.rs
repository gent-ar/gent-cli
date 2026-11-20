use gent_ports::{AgentChatPromptDispatchLedger, ConversationLinkLedger};
use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_PROJECTION_CAPABILITY,
    AGENT_CHAT_PROJECTION_WINDOW_CAPABILITY, AgentChatProjectionDelta, AgentChatProjectionFrame,
    Hello, ProjectionCursor, WireFrame, read_frame, read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, CapabilitySet, ConversationActivityFact, ConversationActivityScope,
    DurableTurnPhase, HostEpoch, PROTOCOL_MAX, PROTOCOL_MIN,
};
use tokio::io::{DuplexStream, duplex};

use super::{RuntimeFacade, facade, root, running};
use crate::api::RuntimeApi;

#[path = "runtime_facade_projection_window_background_tests.rs"]
mod background_tests;

pub(super) async fn connect(runtime: &RuntimeFacade) -> DuplexStream {
    let (mut client, server) = duplex(1024 * 1024);
    tokio::spawn(crate::transport::serve_connection(server, runtime.clone()));
    write_frame(
        &mut client,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![
                AGENT_CHAT_INTENTS_CAPABILITY.into(),
                AGENT_CHAT_PROJECTION_CAPABILITY.into(),
                AGENT_CHAT_PROJECTION_WINDOW_CAPABILITY.into(),
            ]),
        }),
    )
    .await
    .unwrap();
    let WireFrame::Negotiated(negotiated) = read_frame(&mut client).await.unwrap() else {
        panic!("the daemon negotiates");
    };
    assert!(
        negotiated
            .capabilities
            .0
            .iter()
            .any(|capability| capability == AGENT_CHAT_PROJECTION_WINDOW_CAPABILITY)
    );
    client
}

async fn exchange(
    runtime: &RuntimeFacade,
    frame: AgentChatProjectionFrame,
) -> AgentChatProjectionFrame {
    let mut client = connect(runtime).await;
    write_json_frame(&mut client, &frame).await.unwrap();
    read_json_frame(&mut client).await.unwrap()
}

async fn follow(
    runtime: &RuntimeFacade,
    conversation: &AgentChatConversationId,
    after: u64,
    through: u64,
) -> Vec<(u64, AgentChatProjectionDelta)> {
    let mut client = connect(runtime).await;
    write_json_frame(
        &mut client,
        &AgentChatProjectionFrame::FollowConversation {
            request_id: "follow".into(),
            conversation_id: conversation.0.clone(),
            after_cursor: ProjectionCursor { value: after },
        },
    )
    .await
    .unwrap();
    let mut deltas = Vec::new();
    while deltas.last().is_none_or(|(cursor, _)| *cursor < through) {
        let AgentChatProjectionFrame::Delta { delta, .. } =
            read_json_frame(&mut client).await.unwrap()
        else {
            panic!("a follow streams deltas");
        };
        deltas.push((cursor(&delta), delta));
    }
    deltas
}

fn cursor(delta: &AgentChatProjectionDelta) -> u64 {
    match delta {
        AgentChatProjectionDelta::Transcript { cursor, .. }
        | AgentChatProjectionDelta::Activity { cursor, .. }
        | AgentChatProjectionDelta::Lifecycle { cursor, .. }
        | AgentChatProjectionDelta::Catalog { cursor, .. }
        | AgentChatProjectionDelta::Summary { cursor, .. }
        | AgentChatProjectionDelta::UnreadableRecord { cursor, .. } => cursor.value,
    }
}

fn turn_of(delta: &AgentChatProjectionDelta) -> Option<String> {
    match delta {
        AgentChatProjectionDelta::Transcript { event, .. } => Some(event.turn_id.clone()),
        AgentChatProjectionDelta::Activity { fact, .. } => Some(fact.scope().turn_id.clone()),
        _ => None,
    }
}

fn settle(runtime: &RuntimeFacade, prompt: &str) {
    runtime
        .conversation_links()
        .unwrap()
        .settle_agent_chat_prompt_terminal(
            prompt,
            "daemon",
            HostEpoch(1),
            DurableTurnPhase::Completed,
        )
        .unwrap();
}

fn active_turn(
    runtime: &RuntimeFacade,
    conversation: &AgentChatConversationId,
) -> (String, String) {
    let run = runtime
        .conversation_status(&conversation.0)
        .unwrap()
        .runs
        .into_iter()
        .find(|run| run.active_turn_id.is_some())
        .expect("the status read names the running turn");
    (run.run_id, run.active_turn_id.unwrap())
}

#[tokio::test]
async fn a_window_at_a_turn_boundary_carries_the_turns_still_live_there_and_follows_to_head() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let conversation = root(&runtime, "top", ".");
    let first_prompt = running(&runtime, &conversation, "first");
    let (run_id, first_turn) = active_turn(&runtime, &conversation);
    settle(&runtime, &first_prompt);
    let second_prompt = running(&runtime, &conversation, "second");
    let (_, second_turn) = active_turn(&runtime, &conversation);
    runtime
        .conversation_links()
        .unwrap()
        .append_conversation_orchestration_fact(
            &ConversationActivityFact::Recovered {
                scope: ConversationActivityScope {
                    conversation_id: conversation.0.clone(),
                    run_id,
                    turn_id: first_turn.clone(),
                    host_epoch: HostEpoch(1),
                    cursor: 0,
                },
            },
            "late-first-turn-fact",
        )
        .unwrap();
    settle(&runtime, &second_prompt);

    let AgentChatProjectionFrame::ConversationTurnCursors {
        turns,
        next_before_cursor,
        ..
    } = exchange(
        &runtime,
        AgentChatProjectionFrame::ConversationTurnCursorsRequest {
            request_id: "turns".into(),
            conversation_id: conversation.0.clone(),
            before_cursor: None,
            limit: 10,
        },
    )
    .await
    else {
        panic!("turn cursors are returned");
    };
    assert_eq!(
        turns
            .iter()
            .map(|turn| turn.turn_id.as_str())
            .collect::<Vec<_>>(),
        [second_turn.as_str(), first_turn.as_str()]
    );
    assert_eq!(next_before_cursor, None);
    let boundary = turns[0].first_projection_cursor;

    let AgentChatProjectionFrame::ConversationWindow {
        last_turn,
        carried,
        from_cursor,
        ..
    } = exchange(
        &runtime,
        AgentChatProjectionFrame::ConversationWindowRequest {
            request_id: "window".into(),
            conversation_id: conversation.0.clone(),
            from_cursor: ProjectionCursor { value: boundary },
        },
    )
    .await
    else {
        panic!("a window is returned");
    };
    assert_eq!(from_cursor.value, boundary);
    let last_turn = last_turn.expect("the turn before the boundary is named");
    assert_eq!(
        (last_turn.turn_id.as_str(), last_turn.phase),
        (first_turn.as_str(), DurableTurnPhase::Completed)
    );

    let AgentChatProjectionFrame::ConversationSnapshot { snapshot, .. } = exchange(
        &runtime,
        AgentChatProjectionFrame::ConversationSnapshotRequest {
            request_id: "head".into(),
            conversation_id: conversation.0.clone(),
            transcript_limit: 1,
            activity_limit: 1,
        },
    )
    .await
    else {
        panic!("a snapshot names the head cursor");
    };
    let head = follow(&runtime, &conversation, 0, snapshot.cursor.value).await;
    let history = head.iter().take_while(|(cursor, _)| *cursor < boundary);
    let first_turn_history = history
        .filter(|(_, delta)| turn_of(delta).as_deref() == Some(first_turn.as_str()))
        .map(|(_, delta)| delta.clone())
        .collect::<Vec<_>>();
    let carried_turns = carried
        .iter()
        .filter(|delta| turn_of(delta).is_some())
        .cloned()
        .collect::<Vec<_>>();
    assert!(!first_turn_history.is_empty());
    assert_eq!(carried_turns, first_turn_history);
    assert!(carried.iter().all(|delta| cursor(delta) < boundary));

    let window = follow(
        &runtime,
        &conversation,
        boundary - 1,
        head.last().unwrap().0,
    )
    .await;
    assert_eq!(window.first().unwrap().0, boundary);
    assert_eq!(
        window
            .into_iter()
            .map(|(_, delta)| delta)
            .collect::<Vec<_>>(),
        head.into_iter()
            .filter(|(cursor, _)| *cursor >= boundary)
            .map(|(_, delta)| delta)
            .collect::<Vec<_>>()
    );
}
