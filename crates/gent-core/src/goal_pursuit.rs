use gent_types::{
    DurableTurnPhase, GoalDispatchState, GoalRecord, GoalStatus, GoalStatusReason,
    GoalTurnObservation, ProviderFailureClassification,
};

use crate::goal::transition;

pub const MAX_CONTINUATIONS_WITHOUT_PROGRESS: u16 = 3;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GoalPursuitStep {
    Idle,
    Wait,
    Rewake { message_id: String },
    Settle(GoalRecord),
    Admit,
}

#[must_use]
pub fn accounted_goal(current: &GoalRecord, turns: &[GoalTurnObservation], now: u64) -> GoalRecord {
    let mut next = current.clone();
    for turn in turns {
        if turn.ordinal <= next.accounted_through_ordinal {
            continue;
        }
        if !turn.phase.is_terminal() {
            break;
        }
        next.accounted_through_ordinal = turn.ordinal;
        if next.status != GoalStatus::Active {
            continue;
        }
        next.tokens_used = next.tokens_used.saturating_add(turn.tokens);
        match turn.phase {
            DurableTurnPhase::Completed if continues(&next, turn) && turn.tool_calls == 0 => {
                next.turns_without_progress = next.turns_without_progress.saturating_add(1);
            }
            DurableTurnPhase::Completed => next.turns_without_progress = 0,
            DurableTurnPhase::Failed if turn.dispatch != GoalDispatchState::Unprovable => {
                let (status, reason) = failure_status(turn.failure);
                next = transition(&next, status, reason, now);
                next.revision = current.revision;
            }
            _ => {}
        }
    }
    if next == *current {
        return next;
    }
    next.revision = current.revision.saturating_add(1);
    next.updated_at = now.max(current.updated_at);
    next
}

#[must_use]
pub fn next_pursuit_step(
    current: &GoalRecord,
    turns: &[GoalTurnObservation],
    now: u64,
) -> GoalPursuitStep {
    if current.status != GoalStatus::Active {
        return GoalPursuitStep::Idle;
    }
    if let Some(turn) = turns.iter().find(|turn| !turn.phase.is_terminal()) {
        if !continues(current, turn) {
            return GoalPursuitStep::Wait;
        }
        if turn.held {
            return GoalPursuitStep::Settle(transition(
                current,
                GoalStatus::Blocked,
                GoalStatusReason::AdmissionHeld,
                now,
            ));
        }
        if turn.dispatch == GoalDispatchState::AwaitingReadiness {
            return GoalPursuitStep::Rewake {
                message_id: turn.message_id.clone(),
            };
        }
        return GoalPursuitStep::Wait;
    }
    if current.budget_exhausted() {
        return GoalPursuitStep::Settle(transition(
            current,
            GoalStatus::BudgetLimited,
            GoalStatusReason::TokenBudgetExhausted,
            now,
        ));
    }
    if current.turns_without_progress >= MAX_CONTINUATIONS_WITHOUT_PROGRESS {
        return GoalPursuitStep::Settle(transition(
            current,
            GoalStatus::Blocked,
            GoalStatusReason::NoProgressLimit,
            now,
        ));
    }
    GoalPursuitStep::Admit
}

#[must_use]
pub fn blocked_admission(current: &GoalRecord, now: u64) -> Option<GoalRecord> {
    (current.status == GoalStatus::Active).then(|| {
        transition(
            current,
            GoalStatus::Blocked,
            GoalStatusReason::AdmissionHeld,
            now,
        )
    })
}

#[must_use]
pub fn continuation_request_id(goal: &GoalRecord) -> String {
    format!(
        "goal-continuation:{}:{}",
        goal.binding.goal_id, goal.accounted_through_ordinal
    )
}

#[must_use]
pub fn continuation_prompt(goal: &GoalRecord) -> String {
    format!(
        "Continue autonomously toward the active Gent goal.\nObjective: {:?}\nGoal id: {}\n\
         Take the next concrete step now. When the objective is fully achieved, call the \
         `gent_goal_update` tool from the `gent-goal` MCP server with goalId \"{}\" and status \
         \"complete\". If you cannot make further progress without the user, call it with status \
         \"blocked\" and a short note explaining why. Obey Gent permissions and stop for required \
         user decisions.",
        goal.objective, goal.binding.goal_id, goal.binding.goal_id
    )
}

fn continues(goal: &GoalRecord, turn: &GoalTurnObservation) -> bool {
    turn.continuation_of.as_deref() == Some(goal.binding.goal_id.as_str())
}

const fn failure_status(
    failure: Option<ProviderFailureClassification>,
) -> (GoalStatus, GoalStatusReason) {
    match failure {
        Some(ProviderFailureClassification::Authentication) => (
            GoalStatus::Blocked,
            GoalStatusReason::ProviderAuthentication,
        ),
        Some(ProviderFailureClassification::RateLimited) => (
            GoalStatus::UsageLimited,
            GoalStatusReason::ProviderUsageLimit,
        ),
        Some(ProviderFailureClassification::ContextLimit) => {
            (GoalStatus::Blocked, GoalStatusReason::ProviderContextLimit)
        }
        _ => (GoalStatus::Blocked, GoalStatusReason::ProviderFailed),
    }
}
