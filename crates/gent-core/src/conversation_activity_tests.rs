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
fn terminal_root_keeps_live_descendant_visible_and_rejects_late_thinking() {
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
        ConversationActivityFact::Terminal {
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
