//! Pure, conversation-scoped activity state machine for future authoritative hosts.

use std::collections::BTreeSet;

use gent_types::{
    ActivityWork, ActivityWorkKind, CONVERSATION_ACTIVITY_SCHEMA_VERSION, ConversationActivity,
    ConversationActivityFact, ConversationActivityScope, ConversationActivityState, HostEpoch,
    RootActivity, TurnPhase, WorkPhase,
};

/// Pure state retained while applying ordered coordinator facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationActivityProjection {
    snapshot: ConversationActivity,
    root_activity: RootActivity,
    terminal_turns: BTreeSet<String>,
    transition: Option<ConversationActivityState>,
}

/// Result of applying one conversation-scoped fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationActivityUpdate {
    pub projection: ConversationActivityProjection,
    pub applied: bool,
}

impl ConversationActivityProjection {
    /// Creates an empty projection for one immutable conversation/run lineage node.
    #[must_use]
    pub fn new(conversation_id: String, run_id: String, host_epoch: HostEpoch) -> Self {
        Self {
            snapshot: ConversationActivity {
                schema_version: CONVERSATION_ACTIVITY_SCHEMA_VERSION,
                conversation_id,
                run_id,
                host_epoch,
                revision: 0,
                activity_sequence: 0,
                cursor: 0,
                active_turn_id: None,
                root_phase: TurnPhase::Ready,
                state: ConversationActivityState::Idle,
                pending_decision_id: None,
                work: Vec::new(),
                has_error: false,
            },
            root_activity: RootActivity::Idle,
            terminal_turns: BTreeSet::new(),
            transition: None,
        }
    }

    /// Returns the current content-free activity snapshot.
    #[must_use]
    pub fn snapshot(&self) -> &ConversationActivity {
        &self.snapshot
    }
}

/// Applies a strictly newer, same-lineage coordinator fact.
///
/// Terminal root facts dominate later activity for the same turn. A newer turn
/// replaces the active turn before any of its own facts are reduced.
#[must_use]
pub fn project_conversation_activity(
    mut projection: ConversationActivityProjection,
    fact: &ConversationActivityFact,
) -> ConversationActivityUpdate {
    let scope = scope(fact);
    if !matches_lineage(&projection, scope) || scope.cursor <= projection.snapshot.cursor {
        return ConversationActivityUpdate {
            projection,
            applied: false,
        };
    }
    if is_old_turn(&projection, scope, fact) {
        return ConversationActivityUpdate {
            projection,
            applied: false,
        };
    }
    reduce(&mut projection, fact, scope);
    projection.snapshot.cursor = scope.cursor;
    projection.snapshot.revision = projection.snapshot.revision.saturating_add(1);
    projection.snapshot.activity_sequence = projection.snapshot.activity_sequence.saturating_add(1);
    projection.snapshot.state = derive_state(&projection);
    ConversationActivityUpdate {
        projection,
        applied: true,
    }
}

fn scope(fact: &ConversationActivityFact) -> &ConversationActivityScope {
    match fact {
        ConversationActivityFact::TurnStarted { scope }
        | ConversationActivityFact::RootActivity { scope, .. }
        | ConversationActivityFact::RootPhase { scope, .. }
        | ConversationActivityFact::WorkPhase { scope, .. }
        | ConversationActivityFact::DecisionPending { scope, .. }
        | ConversationActivityFact::DecisionSettled { scope, .. }
        | ConversationActivityFact::InterruptRequested { scope }
        | ConversationActivityFact::Recovered { scope }
        | ConversationActivityFact::Terminal { scope, .. } => scope,
    }
}

fn matches_lineage(
    projection: &ConversationActivityProjection,
    scope: &ConversationActivityScope,
) -> bool {
    projection.snapshot.conversation_id == scope.conversation_id
        && projection.snapshot.run_id == scope.run_id
        && projection.snapshot.host_epoch == scope.host_epoch
}

fn is_old_turn(
    projection: &ConversationActivityProjection,
    scope: &ConversationActivityScope,
    fact: &ConversationActivityFact,
) -> bool {
    !matches!(fact, ConversationActivityFact::TurnStarted { .. })
        && projection
            .snapshot
            .active_turn_id
            .as_deref()
            .is_some_and(|turn| turn != scope.turn_id)
}

fn reduce(
    projection: &mut ConversationActivityProjection,
    fact: &ConversationActivityFact,
    scope: &ConversationActivityScope,
) {
    match fact {
        ConversationActivityFact::TurnStarted { .. } => start_turn(projection, &scope.turn_id),
        ConversationActivityFact::RootActivity { activity, .. }
            if active_and_open(projection, scope) =>
        {
            projection.root_activity = *activity;
            projection.transition = None;
        }
        ConversationActivityFact::RootPhase { phase, .. } if active_and_open(projection, scope) => {
            projection.snapshot.root_phase = phase.clone();
            projection.transition = None;
        }
        ConversationActivityFact::WorkPhase {
            work_id,
            kind,
            phase,
            ..
        } if projection.snapshot.active_turn_id.as_deref() == Some(&scope.turn_id) => {
            set_work(projection, work_id, *kind, phase.clone());
        }
        ConversationActivityFact::DecisionPending { decision_id, .. }
            if projection.snapshot.active_turn_id.as_deref() == Some(&scope.turn_id) =>
        {
            projection.snapshot.pending_decision_id = Some(decision_id.clone());
        }
        ConversationActivityFact::DecisionSettled { decision_id, .. }
            if projection.snapshot.pending_decision_id.as_deref() == Some(decision_id) =>
        {
            projection.snapshot.pending_decision_id = None;
        }
        ConversationActivityFact::InterruptRequested { .. }
            if active_and_open(projection, scope) =>
        {
            projection.transition = Some(ConversationActivityState::Interrupting);
        }
        ConversationActivityFact::Recovered { .. } if active_and_open(projection, scope) => {
            projection.transition = Some(ConversationActivityState::Recovering);
        }
        ConversationActivityFact::Terminal { phase, .. }
            if projection.snapshot.active_turn_id.as_deref() == Some(&scope.turn_id) =>
        {
            projection.terminal_turns.insert(scope.turn_id.clone());
            projection.snapshot.root_phase = phase.clone();
            projection.root_activity = RootActivity::Idle;
            projection.transition = None;
        }
        _ => {}
    }
    projection.snapshot.has_error = projection.snapshot.root_phase == TurnPhase::Failed
        || projection.snapshot.root_phase == TurnPhase::Dead
        || projection
            .snapshot
            .work
            .iter()
            .any(|work| work.phase == WorkPhase::Failed);
}

fn start_turn(projection: &mut ConversationActivityProjection, turn_id: &str) {
    projection.snapshot.active_turn_id = Some(turn_id.into());
    projection.snapshot.root_phase = TurnPhase::Processing;
    projection.root_activity = RootActivity::Generating;
    projection.snapshot.pending_decision_id = None;
    projection.snapshot.work.clear();
    projection.transition = None;
}

fn active_and_open(
    projection: &ConversationActivityProjection,
    scope: &ConversationActivityScope,
) -> bool {
    projection.snapshot.active_turn_id.as_deref() == Some(&scope.turn_id)
        && !projection.terminal_turns.contains(&scope.turn_id)
}

fn set_work(
    projection: &mut ConversationActivityProjection,
    work_id: &str,
    kind: ActivityWorkKind,
    phase: WorkPhase,
) {
    if let Some(work) = projection
        .snapshot
        .work
        .iter_mut()
        .find(|work| work.id == work_id)
    {
        work.kind = kind;
        work.phase = phase;
    } else {
        projection.snapshot.work.push(ActivityWork {
            id: work_id.into(),
            kind,
            phase,
        });
    }
    projection
        .snapshot
        .work
        .sort_by(|left, right| left.id.cmp(&right.id));
}

fn derive_state(projection: &ConversationActivityProjection) -> ConversationActivityState {
    if projection.snapshot.pending_decision_id.is_some() {
        return ConversationActivityState::AwaitingUser;
    }
    if let Some(state) = projection.transition {
        return state;
    }
    if projection
        .snapshot
        .work
        .iter()
        .any(|work| work.kind == ActivityWorkKind::Command && work.phase.is_live())
    {
        return ConversationActivityState::WaitingForCommand;
    }
    if projection
        .snapshot
        .work
        .iter()
        .any(|work| work.kind == ActivityWorkKind::Subagent && work.phase.is_live())
    {
        return ConversationActivityState::WaitingForSubagents;
    }
    if matches!(
        projection.snapshot.root_phase,
        TurnPhase::WaitingPermission | TurnPhase::WaitingQuestion
    ) {
        return ConversationActivityState::AwaitingUser;
    }
    if projection.root_activity == RootActivity::Generating
        || matches!(
            projection.snapshot.root_phase,
            TurnPhase::Processing | TurnPhase::Compacting
        )
    {
        return ConversationActivityState::Thinking;
    }
    ConversationActivityState::Idle
}

#[cfg(test)]
#[path = "conversation_activity_tests.rs"]
mod tests;
