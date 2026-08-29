//! Provider-event projection into the pure conversation lifecycle reducer.

use gent_types::{
    ConversationLiveStatus, NormalizedProviderEvent, RootActivity, TurnPhase, WorkPhase,
};

use crate::{LifecycleEvent, LifecycleState, live_status, reduce_lifecycle};

/// Volatile projection state. `last_cursor` makes replay, duplicate delivery, and stale delivery
/// deterministic without requiring access to the event store.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LifecycleProjection {
    pub lifecycle: LifecycleState,
    pub last_cursor: Option<u64>,
    pub active_turn_id: Option<String>,
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

    update_active_turn(&mut state, event);
    for lifecycle_event in lifecycle_events(state.active_turn_id.as_deref(), event) {
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

fn update_active_turn(state: &mut LifecycleProjection, event: &NormalizedProviderEvent) {
    match event {
        NormalizedProviderEvent::TurnStarted { turn_id } => {
            state.active_turn_id = Some(turn_id.clone());
        }
        NormalizedProviderEvent::TurnEnded { turn_id }
            if state.active_turn_id.as_deref() == Some(turn_id) =>
        {
            state.active_turn_id = None;
        }
        _ => {}
    }
}

fn lifecycle_events(
    active_turn_id: Option<&str>,
    event: &NormalizedProviderEvent,
) -> Vec<LifecycleEvent> {
    match event {
        NormalizedProviderEvent::TurnStarted { .. } => vec![
            LifecycleEvent::ErrorCleared,
            LifecycleEvent::RootActivity(RootActivity::Generating),
            LifecycleEvent::RootPhase(TurnPhase::Processing),
        ],
        NormalizedProviderEvent::TurnEnded { turn_id: _ } if active_turn_id.is_none() => {
            vec![
                LifecycleEvent::RootActivity(RootActivity::Idle),
                LifecycleEvent::RootPhase(TurnPhase::Ready),
            ]
        }
        NormalizedProviderEvent::RootActivity { activity } => {
            vec![LifecycleEvent::RootActivity(*activity)]
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
        NormalizedProviderEvent::ProviderFailure { .. } => vec![LifecycleEvent::ErrorRaised],
        NormalizedProviderEvent::TurnEnded { .. }
        | NormalizedProviderEvent::ContextUsage { .. }
        | NormalizedProviderEvent::Output { .. }
        | NormalizedProviderEvent::Thinking { .. }
        | NormalizedProviderEvent::ToolInputDelta { .. }
        | NormalizedProviderEvent::ToolOutputDelta { .. }
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
#[path = "lifecycle_projection_tests.rs"]
mod tests;
