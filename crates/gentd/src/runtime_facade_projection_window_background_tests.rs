use gent_ports::{Ledger, NormalizedSessionBatchLedger, RunLease, RunSessionBinding};
use gent_protocol::{AgentChatProjectionDelta, AgentChatProjectionFrame, ProjectionCursor};
use gent_types::{
    AgentChatConversationId, ConversationActivityFact, ConversationActivityScope, HostEpoch,
    NormalizedProviderEvent, NormalizedSessionBatch, NormalizedSessionLifecycle, WorkPhase,
};

use super::super::{RuntimeFacade, facade, root, running};
use super::{active_turn, cursor, exchange, follow, settle, turn_of};

fn child_fact(
    runtime: &RuntimeFacade,
    conversation: &AgentChatConversationId,
    run_id: &str,
    turn_id: &str,
    key: &str,
    event: NormalizedProviderEvent,
) {
    let activity = matches!(event, NormalizedProviderEvent::ChildStarted { .. }).then(|| {
        ConversationActivityFact::SubagentStarted {
            scope: ConversationActivityScope {
                conversation_id: conversation.0.clone(),
                run_id: run_id.into(),
                turn_id: turn_id.into(),
                host_epoch: HostEpoch(1),
                cursor: 0,
            },
            child_id: "background".into(),
            parent_tool_use_id: "agent-tool".into(),
        }
    });
    runtime
        .conversation_links()
        .unwrap()
        .append_normalized_session_batch(&NormalizedSessionBatch {
            coordinator_id: "daemon".into(),
            conversation_id: conversation.0.clone(),
            run_id: run_id.into(),
            turn_id: turn_id.into(),
            host_epoch: HostEpoch(1),
            lifecycle_event_id: format!("lifecycle-{key}"),
            lifecycle: NormalizedSessionLifecycle::Event { event },
            transcript: None,
            activity_event_id: activity.as_ref().map(|_| format!("activity-{key}")),
            activity,
        })
        .unwrap();
}

async fn window(
    runtime: &RuntimeFacade,
    conversation: &AgentChatConversationId,
    boundary: u64,
) -> Vec<AgentChatProjectionDelta> {
    let AgentChatProjectionFrame::ConversationWindow { carried, .. } = exchange(
        runtime,
        AgentChatProjectionFrame::ConversationWindowRequest {
            request_id: format!("window-{boundary}"),
            conversation_id: conversation.0.clone(),
            from_cursor: ProjectionCursor { value: boundary },
        },
    )
    .await
    else {
        panic!("a window is returned");
    };
    carried
}

#[tokio::test]
async fn a_window_carries_a_settled_turn_while_its_background_subagent_still_runs() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = facade(directory.path()).await;
    let conversation = root(&runtime, "top", ".");
    let first_prompt = running(&runtime, &conversation, "first");
    let (run_id, first_turn) = active_turn(&runtime, &conversation);
    let ledger = runtime.conversation_links().unwrap();
    ledger
        .claim_run_lease(&RunLease {
            run_id: run_id.clone(),
            coordinator_id: "daemon".into(),
            host_epoch: HostEpoch(1),
        })
        .unwrap();
    ledger
        .save_run_session_binding(&RunSessionBinding {
            run_id: run_id.clone(),
            provider_session_id: "session".into(),
        })
        .unwrap();
    child_fact(
        &runtime,
        &conversation,
        &run_id,
        &first_turn,
        "started",
        NormalizedProviderEvent::ChildStarted {
            child_id: "background".into(),
            parent_tool_use_id: "agent-tool".into(),
        },
    );
    settle(&runtime, &first_prompt);
    let second_prompt = running(&runtime, &conversation, "second");
    settle(&runtime, &second_prompt);
    running(&runtime, &conversation, "third");
    let (_, third_turn) = active_turn(&runtime, &conversation);

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
    let boundary = head
        .iter()
        .find(|(_, delta)| turn_of(delta).as_deref() == Some(third_turn.as_str()))
        .unwrap()
        .0;
    let first_turn_history = head
        .iter()
        .filter(|(_, delta)| turn_of(delta).as_deref() == Some(first_turn.as_str()))
        .map(|(_, delta)| delta.clone())
        .collect::<Vec<_>>();
    assert!(first_turn_history.iter().any(|delta| matches!(
        delta,
        AgentChatProjectionDelta::Activity { fact, .. }
            if matches!(**fact, ConversationActivityFact::SubagentStarted { .. })
    )));
    assert!(
        first_turn_history
            .iter()
            .all(|delta| cursor(delta) < boundary)
    );

    let carried = window(&runtime, &conversation, boundary).await;
    let carried_first_turn = carried
        .iter()
        .filter(|delta| turn_of(delta).as_deref() == Some(first_turn.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(carried_first_turn, first_turn_history);

    child_fact(
        &runtime,
        &conversation,
        &run_id,
        &third_turn,
        "terminal",
        NormalizedProviderEvent::ChildTerminal {
            child_id: "background".into(),
            phase: WorkPhase::Done,
        },
    );
    let carried = window(&runtime, &conversation, boundary).await;
    assert!(
        carried
            .iter()
            .all(|delta| turn_of(delta).as_deref() != Some(first_turn.as_str()))
    );
}
