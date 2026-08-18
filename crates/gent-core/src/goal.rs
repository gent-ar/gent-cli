//! Pure state reduction for one durable, provider-neutral user goal.

use gent_types::{GoalBinding, GoalRecord, GoalStatus, GoalTransition};

/// Trusted active scope supplied by a future daemon composition root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GoalControlContext {
    pub conversation_id: String,
    pub run_id: String,
    pub host_epoch: gent_types::HostEpoch,
}

/// In-memory state reconstructed from a durable goal record.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GoalControlState {
    goal: Option<GoalRecord>,
}

impl GoalControlState {
    /// Reconstructs a goal state from an optional durable record.
    #[must_use]
    pub const fn new(goal: Option<GoalRecord>) -> Self {
        Self { goal }
    }

    /// Returns the current goal record, if one has been established.
    #[must_use]
    pub fn goal(&self) -> Option<&GoalRecord> {
        self.goal.as_ref()
    }
}

/// Closed input accepted by the pure goal reducer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GoalControlEvent {
    Create(GoalRecord),
    Transition(GoalTransition),
}

/// A durable write candidate or an explicit no-write outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GoalControlEffect {
    Persist(GoalRecord),
    Unchanged(GoalRecord),
    Rejected(GoalControlRejection),
}

/// Closed rejection reasons that reveal no provider or process state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GoalControlRejection {
    InvalidRecord,
    InvalidTransition,
    GoalAlreadyExists,
    GoalMissing,
    StaleConversation,
    StaleRun,
    StaleHostEpoch,
    BindingMismatch,
    RevisionMismatch,
    TerminalGoal,
    ActiveStatusRequired,
}

/// Reduces one user-owned goal command without I/O, clocks, or provider access.
#[must_use]
pub fn reduce_goal_control(
    state: GoalControlState,
    context: &GoalControlContext,
    event: GoalControlEvent,
) -> (GoalControlState, GoalControlEffect) {
    match event {
        GoalControlEvent::Create(goal) => create_goal(state, context, goal),
        GoalControlEvent::Transition(transition) => transition_goal(state, context, &transition),
    }
}

fn create_goal(
    state: GoalControlState,
    context: &GoalControlContext,
    goal: GoalRecord,
) -> (GoalControlState, GoalControlEffect) {
    if goal.validate().is_err() {
        return (
            state,
            GoalControlEffect::Rejected(GoalControlRejection::InvalidRecord),
        );
    }
    if let Err(rejection) = validate_binding(&goal.binding, context) {
        return (state, GoalControlEffect::Rejected(rejection));
    }
    if goal.revision != 1 || goal.status != GoalStatus::Active {
        return (
            state,
            GoalControlEffect::Rejected(GoalControlRejection::ActiveStatusRequired),
        );
    }
    match state.goal.as_ref() {
        None => {
            let mut next = state;
            next.goal = Some(goal.clone());
            (next, GoalControlEffect::Persist(goal))
        }
        Some(existing) if existing == &goal => (state, GoalControlEffect::Unchanged(goal)),
        Some(_) => (
            state,
            GoalControlEffect::Rejected(GoalControlRejection::GoalAlreadyExists),
        ),
    }
}

fn transition_goal(
    state: GoalControlState,
    context: &GoalControlContext,
    transition: &GoalTransition,
) -> (GoalControlState, GoalControlEffect) {
    if transition.validate().is_err() {
        return (
            state,
            GoalControlEffect::Rejected(GoalControlRejection::InvalidTransition),
        );
    }
    if let Err(rejection) = validate_transition(transition, context) {
        return (state, GoalControlEffect::Rejected(rejection));
    }
    let Some(current) = state.goal.as_ref() else {
        return (
            state,
            GoalControlEffect::Rejected(GoalControlRejection::GoalMissing),
        );
    };
    if current.binding != transition.binding {
        return (
            state,
            GoalControlEffect::Rejected(GoalControlRejection::BindingMismatch),
        );
    }
    if current.revision != transition.expected_revision {
        return (
            state,
            GoalControlEffect::Rejected(GoalControlRejection::RevisionMismatch),
        );
    }
    if current.status.is_terminal() {
        return (
            state,
            GoalControlEffect::Rejected(GoalControlRejection::TerminalGoal),
        );
    }
    if transition.next_status == GoalStatus::Active {
        return (
            state,
            GoalControlEffect::Rejected(GoalControlRejection::ActiveStatusRequired),
        );
    }
    let next = GoalRecord {
        revision: current.revision.saturating_add(1),
        status: transition.next_status,
        ..current.clone()
    };
    let mut next_state = state;
    next_state.goal = Some(next.clone());
    (next_state, GoalControlEffect::Persist(next))
}

fn validate_binding(
    binding: &GoalBinding,
    context: &GoalControlContext,
) -> Result<(), GoalControlRejection> {
    if binding.conversation_id.0 != context.conversation_id {
        return Err(GoalControlRejection::StaleConversation);
    }
    if binding.run_id.0 != context.run_id {
        return Err(GoalControlRejection::StaleRun);
    }
    Ok(())
}

fn validate_transition(
    transition: &GoalTransition,
    context: &GoalControlContext,
) -> Result<(), GoalControlRejection> {
    validate_binding(&transition.binding, context)?;
    if transition.host_epoch != context.host_epoch {
        return Err(GoalControlRejection::StaleHostEpoch);
    }
    Ok(())
}
