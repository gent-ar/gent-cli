use gent_types::{
    AgentChatConversationId, AgentChatRunId, FrozenConversationContext, GOAL_SCHEMA_VERSION,
    GoalBinding, GoalProjection, GoalRecord, GoalStatus,
};

use super::{ClaurstGoalProjection, ClaurstSourceId, ClaurstStartRequest};

fn request() -> ClaurstStartRequest {
    ClaurstStartRequest {
        run_id: "run-1".into(),
        source_id: ClaurstSourceId("source-1".into()),
        turn_id: "turn-1".into(),
        prompt: "continue".into(),
        context: FrozenConversationContext::cleared(AgentChatConversationId("chat-1".into())),
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
    let record = GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: "goal-1".into(),
            conversation_id: AgentChatConversationId("chat-2".into()),
            run_id: AgentChatRunId("run-2".into()),
        },
        revision: 1,
        status: GoalStatus::Active,
        summary: "finish".into(),
    };
    let mut value = request();
    value.goal = Some(ClaurstGoalProjection {
        run_id: "run-2".into(),
        source_id: ClaurstSourceId("source-1".into()),
        goal: GoalProjection::from_active(&record).unwrap(),
    });
    assert!(value.validate().is_err());
}
