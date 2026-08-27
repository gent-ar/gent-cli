use gent_types::{NormalizedProviderEvent, TurnPhase, WorkPhase};

use super::{LifecycleProjection, project_normalized_event, projected_live_status};
use crate::{LifecycleEvent, LifecycleState, reduce_lifecycle};

#[test]
fn projection_ignores_duplicate_and_stale_cursors() {
    let started = NormalizedProviderEvent::TurnStarted {
        turn_id: "turn-1".into(),
    };
    let state = project_normalized_event(LifecycleProjection::default(), 4, &started).state;
    let duplicate = project_normalized_event(
        state.clone(),
        4,
        &NormalizedProviderEvent::TurnEnded {
            turn_id: "turn-1".into(),
        },
    );
    let stale_update = project_normalized_event(state, 3, &started);
    assert!(!duplicate.applied);
    assert_eq!(duplicate.state.lifecycle.root_phase, TurnPhase::Processing);
    assert!(!stale_update.applied);
    assert_eq!(stale_update.state.last_cursor, Some(4));
}

#[test]
fn turn_completion_keeps_detached_child_work_visible() {
    let child = NormalizedProviderEvent::ChildStarted {
        child_id: "child-1".into(),
        parent_tool_use_id: "tool-1".into(),
    };
    let state = project_normalized_event(LifecycleProjection::default(), 1, &child).state;
    let state = project_normalized_event(
        state,
        2,
        &NormalizedProviderEvent::TurnEnded {
            turn_id: "turn-1".into(),
        },
    )
    .state;
    let status = projected_live_status(&state);
    assert_eq!(status.cursor, 2);
    assert!(status.has_live_subagent_work() && status.is_waiting_for_subagents());
}

#[test]
fn terminal_failure_sets_error_and_next_turn_recovers() {
    let failed = NormalizedProviderEvent::CommandTerminal {
        command_id: "command-1".into(),
        phase: WorkPhase::Failed,
    };
    let state = project_normalized_event(LifecycleProjection::default(), 8, &failed).state;
    assert!(projected_live_status(&state).has_error());
    let state = project_normalized_event(
        state,
        9,
        &NormalizedProviderEvent::TurnStarted {
            turn_id: "turn-2".into(),
        },
    )
    .state;
    assert!(!projected_live_status(&state).has_error());
}

#[test]
fn decision_settlement_clears_attention_but_diagnostics_do_not_mutate_state() {
    let state = LifecycleProjection {
        lifecycle: reduce_lifecycle(LifecycleState::default(), LifecycleEvent::AttentionRequired),
        last_cursor: Some(10),
        active_turn_id: None,
    };
    let diagnostic = project_normalized_event(
        state,
        11,
        &NormalizedProviderEvent::TransportDiagnostic {
            classification: "unknownProviderFrame".into(),
        },
    )
    .state;
    assert!(projected_live_status(&diagnostic).needs_attention());
    let settled = project_normalized_event(
        diagnostic,
        12,
        &NormalizedProviderEvent::DecisionSettled {
            decision_id: "decision-1".into(),
        },
    )
    .state;
    assert!(!projected_live_status(&settled).needs_attention());
}

#[test]
fn stale_turn_terminal_cannot_end_the_new_active_turn() {
    let first = project_normalized_event(
        LifecycleProjection::default(),
        1,
        &NormalizedProviderEvent::TurnStarted {
            turn_id: "a".into(),
        },
    )
    .state;
    let second = project_normalized_event(
        first,
        2,
        &NormalizedProviderEvent::TurnStarted {
            turn_id: "b".into(),
        },
    )
    .state;
    let state = project_normalized_event(
        second,
        3,
        &NormalizedProviderEvent::TurnEnded {
            turn_id: "a".into(),
        },
    )
    .state;
    assert_eq!(state.active_turn_id.as_deref(), Some("b"));
    assert!(projected_live_status(&state).is_processing());
}

#[test]
fn waiting_roots_remain_processing_while_detached_work_waits_only_when_ready() {
    for phase in [TurnPhase::WaitingPermission, TurnPhase::WaitingQuestion] {
        let lifecycle =
            reduce_lifecycle(LifecycleState::default(), LifecycleEvent::RootPhase(phase));
        assert!(crate::live_status(&lifecycle, 1).is_processing());
    }
    let lifecycle = reduce_lifecycle(
        LifecycleState::default(),
        LifecycleEvent::ChildPhase {
            child_id: "child".into(),
            phase: WorkPhase::Running,
        },
    );
    let lifecycle = reduce_lifecycle(lifecycle, LifecycleEvent::RootPhase(TurnPhase::Ready));
    assert!(crate::live_status(&lifecycle, 2).is_waiting_for_subagents());
}

#[test]
fn root_activity_and_child_completion_project_without_an_error() {
    let state = project_normalized_event(
        LifecycleProjection::default(),
        1,
        &NormalizedProviderEvent::RootActivity {
            activity: gent_types::RootActivity::Waiting,
        },
    )
    .state;
    let state = project_normalized_event(
        state,
        2,
        &NormalizedProviderEvent::ChildTerminal {
            child_id: "child".into(),
            phase: WorkPhase::Done,
        },
    )
    .state;
    assert!(!projected_live_status(&state).has_error());
}
