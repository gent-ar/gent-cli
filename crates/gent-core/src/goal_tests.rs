use gent_types::{
    AgentChatConversationId, AgentChatRunId, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord,
    GoalStatus, GoalTransition, HostEpoch,
};

use super::{
    GoalControlContext, GoalControlEffect, GoalControlEvent, GoalControlRejection,
    GoalControlState, reduce_goal_control,
};

fn context() -> GoalControlContext {
    GoalControlContext {
        conversation_id: "conversation-1".into(),
        run_id: "run-1".into(),
        host_epoch: HostEpoch(2),
    }
}

fn binding() -> GoalBinding {
    GoalBinding {
        goal_id: "goal-1".into(),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        run_id: AgentChatRunId("run-1".into()),
    }
}

fn record() -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: binding(),
        revision: 1,
        status: GoalStatus::Active,
        summary: "Finish the terminal implementation".into(),
    }
}

#[test]
fn creates_then_settles_an_active_goal_with_a_revision_fence() {
    let (state, effect) = reduce_goal_control(
        GoalControlState::default(),
        &context(),
        GoalControlEvent::Create(record()),
    );
    assert!(matches!(effect, GoalControlEffect::Persist(created) if created.revision == 1));
    let (_, effect) = reduce_goal_control(
        state,
        &context(),
        GoalControlEvent::Transition(GoalTransition {
            binding: binding(),
            expected_revision: 1,
            host_epoch: HostEpoch(2),
            next_status: GoalStatus::Completed,
        }),
    );
    assert!(matches!(effect, GoalControlEffect::Persist(next)
        if next.revision == 2 && next.status == GoalStatus::Completed));
}

#[test]
fn rejects_stale_and_terminal_goal_changes() {
    let active_state = GoalControlState::new(Some(record()));
    let stale_binding = binding();
    let (_, effect) = reduce_goal_control(
        active_state.clone(),
        &context(),
        GoalControlEvent::Transition(GoalTransition {
            binding: stale_binding,
            expected_revision: 1,
            host_epoch: HostEpoch(1),
            next_status: GoalStatus::Failed,
        }),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::StaleHostEpoch)
    );
    let settled = GoalRecord {
        revision: 2,
        status: GoalStatus::Completed,
        ..record()
    };
    let (_, effect) = reduce_goal_control(
        GoalControlState::new(Some(settled)),
        &context(),
        GoalControlEvent::Transition(GoalTransition {
            binding: binding(),
            expected_revision: 2,
            host_epoch: HostEpoch(2),
            next_status: GoalStatus::Failed,
        }),
    );
    assert_eq!(
        effect,
        GoalControlEffect::Rejected(GoalControlRejection::TerminalGoal)
    );
}
