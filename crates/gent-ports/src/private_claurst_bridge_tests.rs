use gent_types::{
    AgentChatConversationId, FrozenConversationContext, GOAL_SCHEMA_VERSION, GoalBinding,
    GoalProjection, GoalRecord, GoalStatus, GoalStatusReason,
};

use super::{
    ClaurstGoalProjection, ClaurstSessionBinding, ClaurstSourceId, ClaurstStartRequest,
    ClaurstSubmitRequest,
};

fn active_goal(conversation_id: &str) -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId(conversation_id.into()),
        },
        revision: 1,
        status: GoalStatus::Active,
        reason: GoalStatusReason::UserSet,
        objective: "finish".into(),
        note: None,
        time_used_seconds: 0,
        active_since: Some(1),
        tokens_used: 0,
        token_budget: None,
        turns_without_progress: 0,
        accounted_through_ordinal: 0,
        created_at: 1,
        updated_at: 1,
    }
}

fn request() -> ClaurstStartRequest {
    ClaurstStartRequest {
        run_id: "run-1".into(),
        source_id: ClaurstSourceId("source-1".into()),
        turn_id: "turn-1".into(),
        prompt: "continue".into(),
        context: FrozenConversationContext::cleared(AgentChatConversationId("chat-1".into())),
        attachments: vec![],
        goal: None,
    }
}

#[test]
fn start_input_accepts_only_complete_gent_owned_values() {
    assert!(request().validate().is_ok());
    let mut invalid = request();
    invalid.prompt = " ".into();
    assert!(invalid.validate().is_err());
}

#[test]
fn start_input_rejects_goal_from_another_run_or_conversation() {
    let record = active_goal("chat-2");
    let mut value = request();
    value.goal = Some(ClaurstGoalProjection {
        run_id: "run-2".into(),
        source_id: ClaurstSourceId("source-1".into()),
        goal: GoalProjection::from_active(&record).unwrap(),
    });
    assert!(value.validate().is_err());
}

#[test]
fn follow_up_input_keeps_the_active_goal_on_its_bound_private_source() {
    let record = active_goal("chat-1");
    let mut request = ClaurstSubmitRequest {
        binding: ClaurstSessionBinding {
            run_id: "run-1".into(),
            source_id: ClaurstSourceId("source-1".into()),
            opaque_session_id: "private-session-1".into(),
        },
        turn_id: "turn-2".into(),
        prompt: "continue".into(),
        attachments: vec![],
        goal: Some(
            ClaurstGoalProjection::from_active_goal(
                "run-1",
                ClaurstSourceId("source-1".into()),
                &record,
            )
            .unwrap(),
        ),
    };
    assert!(request.validate().is_ok());
    request.goal.as_mut().unwrap().source_id = ClaurstSourceId("source-2".into());
    assert!(request.validate().is_err());
}
