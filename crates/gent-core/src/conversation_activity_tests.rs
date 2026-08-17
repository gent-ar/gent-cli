use gent_types::{
    ActivityWorkKind, ConversationActivityFact, ConversationActivityScope,
    ConversationActivityState, HostEpoch, RootActivity, TurnPhase, WorkPhase,
};

use super::{ConversationActivityProjection, project_conversation_activity};

fn scope(cursor: u64, turn_id: &str) -> ConversationActivityScope {
    ConversationActivityScope {
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        turn_id: turn_id.into(),
        host_epoch: HostEpoch(1),
        cursor,
    }
}

fn project(facts: Vec<ConversationActivityFact>) -> ConversationActivityProjection {
    facts.into_iter().fold(
        ConversationActivityProjection::new("conversation".into(), "run".into(), HostEpoch(1)),
        |state, fact| project_conversation_activity(state, &fact).projection,
    )
}

#[test]
fn terminal_root_phase_keeps_live_descendant_visible_and_rejects_late_thinking() {
    let state = project(vec![
        ConversationActivityFact::TurnStarted {
            scope: scope(1, "turn-1"),
        },
        ConversationActivityFact::WorkPhase {
            scope: scope(2, "turn-1"),
            work_id: "child".into(),
            kind: ActivityWorkKind::Subagent,
            phase: WorkPhase::Running,
        },
        ConversationActivityFact::RootPhase {
            scope: scope(3, "turn-1"),
            phase: TurnPhase::Ready,
        },
        ConversationActivityFact::RootActivity {
            scope: scope(4, "turn-1"),
            activity: RootActivity::Generating,
        },
    ]);
    assert_eq!(
        state.snapshot().state,
        ConversationActivityState::WaitingForSubagents
    );
    assert_eq!(state.snapshot().cursor, 4);
}

#[test]
fn decision_has_priority_and_new_turn_rejects_old_facts() {
    let state = project(vec![
        ConversationActivityFact::TurnStarted {
            scope: scope(1, "old"),
        },
        ConversationActivityFact::DecisionPending {
            scope: scope(2, "old"),
            decision_id: "decision".into(),
        },
        ConversationActivityFact::TurnStarted {
            scope: scope(3, "new"),
        },
        ConversationActivityFact::WorkPhase {
            scope: scope(4, "old"),
            work_id: "old-child".into(),
            kind: ActivityWorkKind::Subagent,
            phase: WorkPhase::Running,
        },
    ]);
    assert_eq!(state.snapshot().active_turn_id.as_deref(), Some("new"));
    assert_eq!(state.snapshot().state, ConversationActivityState::Thinking);
    assert_eq!(state.snapshot().cursor, 3);
}

#[test]
fn state_transitions_are_cursor_and_revision_monotonic() {
    let state = project(vec![
        ConversationActivityFact::TurnStarted {
            scope: scope(1, "turn"),
        },
        ConversationActivityFact::InterruptRequested {
            scope: scope(2, "turn"),
        },
        ConversationActivityFact::Recovered {
            scope: scope(3, "turn"),
        },
    ]);
    assert_eq!(
        state.snapshot().state,
        ConversationActivityState::Recovering
    );
    assert_eq!(state.snapshot().revision, 3);
    assert_eq!(state.snapshot().activity_sequence, 3);
}

#[test]
fn facts_from_a_superseded_host_epoch_are_rejected() {
    let mut stale_scope = scope(1, "turn");
    stale_scope.host_epoch = HostEpoch(2);
    let update = project_conversation_activity(
        ConversationActivityProjection::new("conversation".into(), "run".into(), HostEpoch(1)),
        &ConversationActivityFact::TurnStarted { scope: stale_scope },
    );
    assert!(!update.applied);
    assert_eq!(update.projection.snapshot().cursor, 0);
}

#[test]
fn durable_record_restores_terminal_dominance() {
    let state = project(vec![
        ConversationActivityFact::TurnStarted {
            scope: scope(1, "turn"),
        },
        ConversationActivityFact::Terminal {
            scope: scope(2, "turn"),
            phase: TurnPhase::Ready,
        },
    ]);
    let restored = ConversationActivityProjection::from_record(state.record());
    let update = project_conversation_activity(
        restored,
        &ConversationActivityFact::RootActivity {
            scope: scope(3, "turn"),
            activity: RootActivity::Generating,
        },
    );
    assert_eq!(
        update.projection.snapshot().state,
        ConversationActivityState::Idle
    );
}

#[test]
fn terminal_turn_cannot_be_started_again_by_a_late_fact() {
    let state = project(vec![
        ConversationActivityFact::TurnStarted {
            scope: scope(1, "old"),
        },
        ConversationActivityFact::Terminal {
            scope: scope(2, "old"),
            phase: TurnPhase::Ready,
        },
        ConversationActivityFact::TurnStarted {
            scope: scope(3, "new"),
        },
        ConversationActivityFact::TurnStarted {
            scope: scope(4, "old"),
        },
    ]);
    assert_eq!(state.snapshot().active_turn_id.as_deref(), Some("new"));
    assert_eq!(state.snapshot().cursor, 3);
}

#[test]
fn terminal_work_cannot_be_revived_by_a_late_live_fact() {
    let state = project(vec![
        ConversationActivityFact::TurnStarted {
            scope: scope(1, "turn"),
        },
        ConversationActivityFact::WorkPhase {
            scope: scope(2, "turn"),
            work_id: "child".into(),
            kind: ActivityWorkKind::Subagent,
            phase: WorkPhase::Running,
        },
        ConversationActivityFact::WorkPhase {
            scope: scope(3, "turn"),
            work_id: "child".into(),
            kind: ActivityWorkKind::Subagent,
            phase: WorkPhase::Done,
        },
        ConversationActivityFact::WorkPhase {
            scope: scope(4, "turn"),
            work_id: "child".into(),
            kind: ActivityWorkKind::Subagent,
            phase: WorkPhase::Running,
        },
    ]);

    assert_eq!(state.snapshot().state, ConversationActivityState::Thinking);
    assert_eq!(state.snapshot().work[0].phase, WorkPhase::Done);
    assert_eq!(state.snapshot().cursor, 4);
}

#[test]
fn restored_projection_keeps_terminal_work_terminal() {
    let state = project(vec![
        ConversationActivityFact::TurnStarted {
            scope: scope(1, "turn"),
        },
        ConversationActivityFact::WorkPhase {
            scope: scope(2, "turn"),
            work_id: "command".into(),
            kind: ActivityWorkKind::Command,
            phase: WorkPhase::Interrupted,
        },
    ]);
    let update = project_conversation_activity(
        ConversationActivityProjection::from_record(state.record()),
        &ConversationActivityFact::WorkPhase {
            scope: scope(3, "turn"),
            work_id: "command".into(),
            kind: ActivityWorkKind::Command,
            phase: WorkPhase::Running,
        },
    );

    assert_eq!(
        update.projection.snapshot().work[0].phase,
        WorkPhase::Interrupted
    );
    assert_eq!(
        update.projection.snapshot().state,
        ConversationActivityState::Thinking
    );
}

#[test]
fn decision_wins_interrupt_and_terminal_races_until_it_settles() {
    let state = project(vec![
        ConversationActivityFact::TurnStarted {
            scope: scope(1, "turn"),
        },
        ConversationActivityFact::WorkPhase {
            scope: scope(2, "turn"),
            work_id: "command".into(),
            kind: ActivityWorkKind::Command,
            phase: WorkPhase::Running,
        },
        ConversationActivityFact::DecisionPending {
            scope: scope(3, "turn"),
            decision_id: "decision".into(),
        },
        ConversationActivityFact::InterruptRequested {
            scope: scope(4, "turn"),
        },
        ConversationActivityFact::Terminal {
            scope: scope(5, "turn"),
            phase: TurnPhase::Interrupted,
        },
    ]);
    assert_eq!(
        state.snapshot().state,
        ConversationActivityState::AwaitingUser
    );

    let settled = project_conversation_activity(
        state,
        &ConversationActivityFact::DecisionSettled {
            scope: scope(6, "turn"),
            decision_id: "decision".into(),
        },
    );
    assert_eq!(
        settled.projection.snapshot().state,
        ConversationActivityState::WaitingForCommand
    );
}
