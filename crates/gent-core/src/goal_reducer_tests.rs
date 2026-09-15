use gent_types::{
    DurableTurnPhase, GoalDispatchState, GoalStatus, GoalStatusReason, GoalTurnObservation,
    ProviderFailureClassification,
};

use crate::goal_tests::active;
use crate::{
    GoalPursuitStep, GoalUserCommand, MAX_CONTINUATIONS_WITHOUT_PROGRESS, accounted_goal,
    apply_user_command, next_pursuit_step,
};

fn turn(ordinal: u64, phase: DurableTurnPhase) -> GoalTurnObservation {
    GoalTurnObservation {
        ordinal,
        message_id: format!("message-{ordinal}"),
        run_id: "run-1".into(),
        receipt_id: format!("receipt-{ordinal}"),
        turn_id: format!("turn-{ordinal}"),
        phase,
        dispatch: if phase.is_terminal() {
            GoalDispatchState::Settled
        } else {
            GoalDispatchState::InFlight
        },
        continuation_of: None,
        held: false,
        tokens: 0,
        tool_calls: 0,
        failure: None,
    }
}

fn continuation(ordinal: u64, phase: DurableTurnPhase) -> GoalTurnObservation {
    GoalTurnObservation {
        continuation_of: Some("goal-1".into()),
        ..turn(ordinal, phase)
    }
}

#[test]
fn accounting_adds_tokens_and_advances_through_the_terminal_prefix() {
    let goal = active(100);
    let turns = [
        GoalTurnObservation {
            tokens: 40,
            tool_calls: 2,
            ..continuation(3, DurableTurnPhase::Completed)
        },
        turn(4, DurableTurnPhase::Active),
        GoalTurnObservation {
            tokens: 900,
            ..turn(5, DurableTurnPhase::Completed)
        },
    ];
    let next = accounted_goal(&goal, &turns, 120);
    assert_eq!(next.accounted_through_ordinal, 3);
    assert_eq!(next.tokens_used, 40);
    assert_eq!(next.revision, 2);
    assert_eq!(accounted_goal(&next, &turns, 130), next);
}

#[test]
fn continuation_turns_without_tool_work_count_toward_the_progress_ceiling() {
    let mut goal = active(100);
    for ordinal in 3..3 + u64::from(MAX_CONTINUATIONS_WITHOUT_PROGRESS) {
        goal = accounted_goal(
            &goal,
            &[continuation(ordinal, DurableTurnPhase::Completed)],
            110,
        );
        if goal.turns_without_progress < MAX_CONTINUATIONS_WITHOUT_PROGRESS {
            assert_eq!(next_pursuit_step(&goal, &[], 111), GoalPursuitStep::Admit);
        }
    }
    let GoalPursuitStep::Settle(blocked) = next_pursuit_step(&goal, &[], 120) else {
        panic!("a goal past its progress ceiling must settle");
    };
    assert_eq!(blocked.status, GoalStatus::Blocked);
    assert_eq!(blocked.reason, GoalStatusReason::NoProgressLimit);
    let user_turn = accounted_goal(&goal, &[turn(20, DurableTurnPhase::Completed)], 130);
    assert_eq!(user_turn.turns_without_progress, 0);
}

#[test]
fn provider_failures_settle_with_typed_reasons_but_restart_loss_does_not() {
    let goal = active(100);
    let cases = [
        (
            Some(ProviderFailureClassification::Authentication),
            GoalStatus::Blocked,
            GoalStatusReason::ProviderAuthentication,
        ),
        (
            Some(ProviderFailureClassification::RateLimited),
            GoalStatus::UsageLimited,
            GoalStatusReason::ProviderUsageLimit,
        ),
        (None, GoalStatus::Blocked, GoalStatusReason::ProviderFailed),
    ];
    for (failure, status, reason) in cases {
        let failed = GoalTurnObservation {
            failure,
            ..continuation(3, DurableTurnPhase::Failed)
        };
        let next = accounted_goal(&goal, &[failed], 150);
        assert_eq!((next.status, next.reason), (status, reason));
        assert_eq!((next.time_used_seconds, next.active_since), (50, None));
        assert_eq!(next_pursuit_step(&next, &[], 151), GoalPursuitStep::Idle);
    }
    let unprovable = GoalTurnObservation {
        dispatch: GoalDispatchState::Unprovable,
        ..continuation(3, DurableTurnPhase::Failed)
    };
    let recovered = accounted_goal(&goal, &[unprovable], 150);
    assert_eq!(recovered.status, GoalStatus::Active);
    assert_eq!(
        next_pursuit_step(&recovered, &[], 151),
        GoalPursuitStep::Admit
    );
}

#[test]
fn interrupted_turns_neither_pause_nor_count_against_the_goal() {
    let goal = active(100);
    let next = accounted_goal(
        &goal,
        &[continuation(3, DurableTurnPhase::Interrupted)],
        120,
    );
    assert_eq!(next.status, GoalStatus::Active);
    assert_eq!(next.turns_without_progress, 0);
}

#[test]
fn a_waiting_user_prompt_always_runs_before_the_next_continuation() {
    let goal = active(100);
    let queued = GoalTurnObservation {
        dispatch: GoalDispatchState::Pending,
        ..turn(3, DurableTurnPhase::Active)
    };
    assert_eq!(
        next_pursuit_step(&goal, &[queued], 110),
        GoalPursuitStep::Wait
    );
}

#[test]
fn a_held_continuation_blocks_instead_of_spinning() {
    let goal = active(100);
    let held = GoalTurnObservation {
        dispatch: GoalDispatchState::AwaitingReadiness,
        held: true,
        ..continuation(3, DurableTurnPhase::Active)
    };
    let GoalPursuitStep::Settle(blocked) = next_pursuit_step(&goal, &[held], 110) else {
        panic!("held continuation must settle the goal");
    };
    assert_eq!(blocked.reason, GoalStatusReason::AdmissionHeld);
    let unreleased = GoalTurnObservation {
        dispatch: GoalDispatchState::AwaitingReadiness,
        ..continuation(3, DurableTurnPhase::Active)
    };
    assert_eq!(
        next_pursuit_step(&goal, &[unreleased], 110),
        GoalPursuitStep::Rewake {
            message_id: "message-3".into()
        }
    );
}

#[test]
fn an_exhausted_budget_settles_before_another_continuation() {
    let goal = active(100);
    let spent = accounted_goal(
        &goal,
        &[GoalTurnObservation {
            tokens: 1_000,
            tool_calls: 1,
            ..continuation(3, DurableTurnPhase::Completed)
        }],
        110,
    );
    let GoalPursuitStep::Settle(limited) = next_pursuit_step(&spent, &[], 111) else {
        panic!("spent budget must settle the goal");
    };
    assert_eq!(limited.status, GoalStatus::BudgetLimited);
    assert_eq!(limited.reason, GoalStatusReason::TokenBudgetExhausted);
}

#[test]
fn a_paused_goal_does_not_schedule_but_its_running_turn_is_left_alone() {
    let paused = apply_user_command(&active(100), 1, GoalUserCommand::Pause, 110).unwrap();
    let running = continuation(3, DurableTurnPhase::Active);
    assert_eq!(
        next_pursuit_step(&paused, &[running], 120),
        GoalPursuitStep::Idle
    );
}
