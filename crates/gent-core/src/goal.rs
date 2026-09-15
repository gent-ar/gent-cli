//! Pure state reduction for one durable, provider-neutral user goal.

use gent_types::{
    AgentChatConversationId, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord, GoalReportOutcome,
    GoalStatus, GoalStatusReason,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GoalRejection {
    InvalidValue,
    Missing,
    RevisionMismatch,
    NotActive,
    NotResumable,
    BudgetExhausted,
    NoActiveTurn,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalDraft {
    pub goal_id: String,
    pub conversation_id: AgentChatConversationId,
    pub objective: String,
    pub token_budget: Option<u64>,
    pub accounted_through_ordinal: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GoalUserCommand {
    Pause,
    Resume { accounted_through_ordinal: u64 },
    Clear,
}

pub fn create_goal(draft: GoalDraft, now: u64) -> Result<GoalRecord, GoalRejection> {
    let goal = GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: GoalBinding {
            goal_id: draft.goal_id,
            conversation_id: draft.conversation_id,
        },
        revision: 1,
        status: GoalStatus::Active,
        reason: GoalStatusReason::UserSet,
        objective: draft.objective,
        note: None,
        time_used_seconds: 0,
        active_since: Some(now),
        tokens_used: 0,
        token_budget: draft.token_budget,
        turns_without_progress: 0,
        accounted_through_ordinal: draft.accounted_through_ordinal,
        created_at: now,
        updated_at: now,
    };
    goal.validate().map_err(|_| GoalRejection::InvalidValue)?;
    Ok(goal)
}

#[must_use]
pub fn replaced_goal(current: &GoalRecord, now: u64) -> Option<GoalRecord> {
    (!current.status.is_terminal()).then(|| {
        transition(
            current,
            GoalStatus::Cleared,
            GoalStatusReason::UserReplaced,
            now,
        )
    })
}

pub fn apply_user_command(
    current: &GoalRecord,
    expected_revision: u64,
    command: GoalUserCommand,
    now: u64,
) -> Result<GoalRecord, GoalRejection> {
    if current.revision != expected_revision {
        return Err(GoalRejection::RevisionMismatch);
    }
    match command {
        GoalUserCommand::Pause if current.status == GoalStatus::Active => Ok(transition(
            current,
            GoalStatus::Paused,
            GoalStatusReason::UserPaused,
            now,
        )),
        GoalUserCommand::Pause => Err(GoalRejection::NotActive),
        GoalUserCommand::Resume { .. } if !current.status.is_resumable() => {
            Err(GoalRejection::NotResumable)
        }
        GoalUserCommand::Resume { .. } if current.budget_exhausted() => {
            Err(GoalRejection::BudgetExhausted)
        }
        GoalUserCommand::Resume {
            accounted_through_ordinal,
        } => {
            let mut next = transition(
                current,
                GoalStatus::Active,
                GoalStatusReason::UserResumed,
                now,
            );
            next.turns_without_progress = 0;
            next.note = None;
            next.accounted_through_ordinal = accounted_through_ordinal;
            Ok(next)
        }
        GoalUserCommand::Clear if current.status == GoalStatus::Cleared => {
            Err(GoalRejection::NotActive)
        }
        GoalUserCommand::Clear => Ok(transition(
            current,
            GoalStatus::Cleared,
            GoalStatusReason::UserCleared,
            now,
        )),
    }
}

#[must_use]
pub fn stopped_goal(current: &GoalRecord, now: u64) -> Option<GoalRecord> {
    (current.status == GoalStatus::Active).then(|| {
        transition(
            current,
            GoalStatus::Paused,
            GoalStatusReason::UserStopped,
            now,
        )
    })
}

pub fn reported_goal(
    current: &GoalRecord,
    outcome: GoalReportOutcome,
    note: Option<String>,
    now: u64,
) -> Result<GoalRecord, GoalRejection> {
    if current.status != GoalStatus::Active {
        return Err(GoalRejection::NotActive);
    }
    let (status, reason) = match outcome {
        GoalReportOutcome::Complete => (GoalStatus::Complete, GoalStatusReason::ModelCompleted),
        GoalReportOutcome::Blocked => (GoalStatus::Blocked, GoalStatusReason::ModelBlocked),
    };
    let mut next = transition(current, status, reason, now);
    next.note = note;
    next.validate().map_err(|_| GoalRejection::InvalidValue)?;
    Ok(next)
}

pub(crate) fn transition(
    current: &GoalRecord,
    status: GoalStatus,
    reason: GoalStatusReason,
    now: u64,
) -> GoalRecord {
    let now = now.max(current.updated_at);
    let mut next = current.clone();
    next.revision = current.revision.saturating_add(1);
    next.status = status;
    next.reason = reason;
    next.updated_at = now;
    if status == GoalStatus::Active {
        next.active_since = current.active_since.or(Some(now));
    } else {
        next.time_used_seconds = current.time_used_at(now);
        next.active_since = None;
    }
    next
}
