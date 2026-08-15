//! Provider-event projection into the pure conversation lifecycle reducer.

use gent_types::{ConversationLiveStatus, NormalizedProviderEvent, TurnPhase, WorkPhase};

use crate::{LifecycleEvent, LifecycleState, live_status, reduce_lifecycle};

/// Volatile projection state. `last_cursor` makes replay, duplicate delivery, and stale delivery
/// deterministic without requiring access to the event store.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LifecycleProjection {
    pub lifecycle: LifecycleState,
    pub last_cursor: Option<u64>,
}

/// Result of attempting to apply one durable provider event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionUpdate {
    pub state: LifecycleProjection,
    pub applied: bool,
}

/// Applies a strictly newer event cursor to the lifecycle projection.
///
/// Equal and lower cursors are ignored so resume delivery cannot regress state or duplicate an
/// already-applied transition.
#[must_use]
pub fn project_normalized_event(
    mut state: LifecycleProjection,
    cursor: u64,
    event: &NormalizedProviderEvent,
) -> ProjectionUpdate {
    if state
        .last_cursor
        .is_some_and(|last_cursor| cursor <= last_cursor)
    {
        return ProjectionUpdate {
            state,
            applied: false,
        };
    }

    for lifecycle_event in lifecycle_events(event) {
        state.lifecycle = reduce_lifecycle(state.lifecycle, lifecycle_event);
    }
    state.last_cursor = Some(cursor);
    ProjectionUpdate {
        state,
        applied: true,
    }
}

/// Derives client-facing status from the latest accepted durable cursor.
#[must_use]
pub fn projected_live_status(state: &LifecycleProjection) -> ConversationLiveStatus {
    live_status(&state.lifecycle, state.last_cursor.unwrap_or_default())
}

fn lifecycle_events(event: &NormalizedProviderEvent) -> Vec<LifecycleEvent> {
    match event {
        NormalizedProviderEvent::TurnStarted { .. } => vec![
            LifecycleEvent::ErrorCleared,
            LifecycleEvent::RootPhase(TurnPhase::Processing),
        ],
        NormalizedProviderEvent::TurnEnded { .. } => {
            vec![LifecycleEvent::RootPhase(TurnPhase::Ready)]
        }
        NormalizedProviderEvent::ChildStarted { child_id, .. } => {
            vec![LifecycleEvent::ChildPhase {
                child_id: child_id.clone(),
                phase: WorkPhase::Running,
            }]
        }
        NormalizedProviderEvent::ChildTerminal { child_id, phase } => terminal_events(
            LifecycleEvent::ChildPhase {
                child_id: child_id.clone(),
                phase: phase.clone(),
            },
            phase,
        ),
        NormalizedProviderEvent::CommandTerminal { command_id, phase } => terminal_events(
            LifecycleEvent::CommandPhase {
                command_id: command_id.clone(),
                phase: phase.clone(),
            },
            phase,
        ),
        NormalizedProviderEvent::DecisionSettled { .. } => vec![LifecycleEvent::AttentionCleared],
        NormalizedProviderEvent::Output { .. }
        | NormalizedProviderEvent::TransportDiagnostic { .. } => Vec::new(),
    }
}

fn terminal_events(event: LifecycleEvent, phase: &WorkPhase) -> Vec<LifecycleEvent> {
    let mut events = vec![event];
    if *phase == WorkPhase::Failed {
        events.push(LifecycleEvent::ErrorRaised);
    }
    events
}

#[cfg(test)]
mod tests {
    use gent_types::{NormalizedProviderEvent, WorkPhase};

    use super::{LifecycleProjection, project_normalized_event, projected_live_status};
    use crate::{LifecycleEvent, LifecycleState, TurnPhase, reduce_lifecycle};

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
        let child_started = NormalizedProviderEvent::ChildStarted {
            child_id: "child-1".into(),
            parent_tool_use_id: "tool-1".into(),
        };
        let state =
            project_normalized_event(LifecycleProjection::default(), 1, &child_started).state;
        let state = project_normalized_event(
            state,
            2,
            &NormalizedProviderEvent::TurnEnded {
                turn_id: "turn-1".into(),
            },
        )
        .state;

        let status = projected_live_status(&state);
        assert_eq!(status.snapshot_cursor, 2);
        assert!(status.has_live_subagent_work);
        assert!(status.is_waiting_for_subagents);
    }

    #[test]
    fn terminal_failure_sets_error_and_next_turn_recovers() {
        let failed = NormalizedProviderEvent::CommandTerminal {
            command_id: "command-1".into(),
            phase: WorkPhase::Failed,
        };
        let state = project_normalized_event(LifecycleProjection::default(), 8, &failed).state;
        assert!(projected_live_status(&state).has_error);

        let state = project_normalized_event(
            state,
            9,
            &NormalizedProviderEvent::TurnStarted {
                turn_id: "turn-2".into(),
            },
        )
        .state;
        assert!(!projected_live_status(&state).has_error);
    }

    #[test]
    fn decision_settlement_clears_attention_but_diagnostics_do_not_mutate_state() {
        let lifecycle =
            reduce_lifecycle(LifecycleState::default(), LifecycleEvent::AttentionRequired);
        let state = LifecycleProjection {
            lifecycle,
            last_cursor: Some(10),
        };
        let diagnostic = project_normalized_event(
            state,
            11,
            &NormalizedProviderEvent::TransportDiagnostic {
                classification: "unknownProviderFrame".into(),
            },
        )
        .state;
        assert!(projected_live_status(&diagnostic).needs_attention);

        let settled = project_normalized_event(
            diagnostic,
            12,
            &NormalizedProviderEvent::DecisionSettled {
                decision_id: "decision-1".into(),
            },
        )
        .state;
        assert!(!projected_live_status(&settled).needs_attention);
    }
}
