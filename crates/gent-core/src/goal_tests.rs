use gent_types::{
    AgentChatConversationId, AgentChatRunId, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord,
    GoalStatus, GoalTransition, HostEpoch,
};

use super::{
    ActiveGoalRejection, ActiveGoalSelection, GoalControlContext, GoalControlEffect,
    GoalControlEvent, GoalControlRejection, GoalControlState, reduce_goal_control,
    select_active_goal,
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

#[test]
fn selection_omits_terminal_and_stale_goals_but_uses_the_current_exact_revision() {
    let current = GoalRecord {
        revision: 4,
        ..record()
    };
    let terminal = GoalRecord {
        revision: 2,
        status: GoalStatus::Completed,
        ..record()
    };
    let stale = GoalRecord {
        binding: GoalBinding {
            run_id: AgentChatRunId("old-run".into()),
            ..binding()
        },
        ..record()
    };
    assert_eq!(
        select_active_goal(&[terminal, stale, current], "conversation-1", "run-1"),
        ActiveGoalSelection::Goal(
            gent_types::GoalProjection::from_active(&GoalRecord {
                revision: 4,
                ..record()
            })
            .unwrap()
        )
    );
}

#[test]
fn selection_rejects_ambiguous_or_malformed_matching_active_goals() {
    let other = GoalRecord {
        binding: GoalBinding {
            goal_id: "goal-2".into(),
            ..binding()
        },
        ..record()
    };
    assert_eq!(
        select_active_goal(&[record(), other], "conversation-1", "run-1"),
        ActiveGoalSelection::Rejected(ActiveGoalRejection::AmbiguousActiveGoals)
    );
    let malformed = GoalRecord {
        summary: String::new(),
        ..record()
    };
    assert_eq!(
        select_active_goal(&[malformed], "conversation-1", "run-1"),
        ActiveGoalSelection::Rejected(ActiveGoalRejection::InvalidActiveGoal)
    );
}
