use gent_ports::{
    ClaurstCheckpoint, ClaurstDrainBatch, ClaurstDrainRequest, ClaurstFactValue,
    ClaurstGoalProjection, ClaurstNormalizedFact, ClaurstSessionBinding, ClaurstSourceId,
    ClaurstTerminal, PrivateClaurstBridge,
};
use gent_testkit::FakePrivateClaurstBridge;
use gent_types::{
    AgentChatConversationId, AgentChatRunId, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord,
    GoalStatus, NormalizedProviderEvent,
};

fn source() -> ClaurstSourceId {
    ClaurstSourceId("source-a".into())
}

fn binding() -> ClaurstSessionBinding {
    ClaurstSessionBinding {
        run_id: "run-a".into(),
        source_id: source(),
        opaque_session_id: "private-session-a".into(),
    }
}

fn request(after_cursor: u64, limit: u16) -> ClaurstDrainRequest {
    ClaurstDrainRequest {
        run_id: "run-a".into(),
        source_id: source(),
        after_cursor,
        limit,
    }
}

fn active_goal() -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-a".into(),
            conversation_id: AgentChatConversationId("conversation-a".into()),
            run_id: AgentChatRunId("run-a".into()),
        },
        revision: 2,
        status: GoalStatus::Active,
        summary: "Finish safely".into(),
    }
}

#[test]
fn private_goal_dto_maps_only_a_valid_active_goal_without_bridge_configuration() {
    let projection = ClaurstGoalProjection::from_active_goal(source(), &active_goal()).unwrap();
    assert_eq!(projection.run_id, "run-a");
    assert_eq!(projection.goal.binding().goal_id, "goal-a");
    assert_eq!(projection.goal.revision(), 2);

    let terminal = GoalRecord {
        status: GoalStatus::Completed,
        ..active_goal()
    };
    assert!(ClaurstGoalProjection::from_active_goal(source(), &terminal).is_err());
}

#[tokio::test]
async fn bridge_drains_ordered_normalized_facts_and_terminal_once() {
    let bridge = FakePrivateClaurstBridge::default();
    bridge.bind_session(binding()).await.unwrap();
    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![ClaurstNormalizedFact {
            source_id: source(),
            cursor: 4,
            value: ClaurstFactValue::Event(NormalizedProviderEvent::Output {
                text: "safe normalized reply".into(),
                is_partial: false,
            }),
        }],
        checkpoint: Some(ClaurstCheckpoint {
            run_id: "run-a".into(),
            source_id: source(),
            cursor: 4,
            state_digest_sha256: "digest".into(),
        }),
        session_binding: None,
        terminal: Some(ClaurstTerminal::Completed),
    });

    let batch = bridge.drain(request(3, 1)).await.unwrap();
    assert_eq!(batch.facts.len(), 1);
    assert!(matches!(batch.terminal, Some(ClaurstTerminal::Completed)));
    assert_eq!(bridge.requests(), vec![request(3, 1)]);
    assert!(bridge.drain(request(4, 1)).await.is_err());
}

#[tokio::test]
async fn bridge_rejects_unbounded_or_opaque_session_echoes() {
    let bridge = FakePrivateClaurstBridge::default();
    bridge.bind_session(binding()).await.unwrap();
    assert!(bridge.drain(request(0, 65)).await.is_err());

    bridge.push_batch(ClaurstDrainBatch {
        facts: vec![ClaurstNormalizedFact {
            source_id: source(),
            cursor: 1,
            value: ClaurstFactValue::Event(NormalizedProviderEvent::Output {
                text: "private-session-a".into(),
                is_partial: false,
            }),
        }],
        checkpoint: None,
        session_binding: None,
        terminal: None,
    });
    assert!(bridge.drain(request(0, 1)).await.is_err());
}
