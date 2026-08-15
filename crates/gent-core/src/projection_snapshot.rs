//! Pure conversion between an in-memory lifecycle projection and its durable value snapshot.

use gent_types::RunLifecycleProjection;

use crate::{LifecycleProjection, LifecycleState};

/// Exports the complete pure state required to continue projection after process restart.
#[must_use]
pub fn snapshot_projection(state: &LifecycleProjection) -> RunLifecycleProjection {
    RunLifecycleProjection {
        cursor: state.last_cursor.unwrap_or_default(),
        active_turn_id: state.active_turn_id.clone(),
        root_phase: state.lifecycle.root_phase.clone(),
        children: state.lifecycle.children.clone(),
        commands: state.lifecycle.commands.clone(),
        needs_attention: state.lifecycle.needs_attention,
        has_error: state.lifecycle.has_error,
    }
}

/// Restores a projection without re-running prior events or consulting infrastructure.
#[must_use]
pub fn restore_projection(snapshot: &RunLifecycleProjection) -> LifecycleProjection {
    LifecycleProjection {
        lifecycle: LifecycleState {
            root_phase: snapshot.root_phase.clone(),
            children: snapshot.children.clone(),
            commands: snapshot.commands.clone(),
            needs_attention: snapshot.needs_attention,
            has_error: snapshot.has_error,
        },
        last_cursor: Some(snapshot.cursor),
        active_turn_id: snapshot.active_turn_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use gent_types::NormalizedProviderEvent;

    use super::{restore_projection, snapshot_projection};
    use crate::{LifecycleProjection, TurnPhase, project_normalized_event};

    #[test]
    fn snapshot_restores_active_turn_and_work_for_following_events() {
        let started = project_normalized_event(
            LifecycleProjection::default(),
            4,
            &NormalizedProviderEvent::TurnStarted {
                turn_id: "turn-a".into(),
            },
        )
        .state;
        let restored = restore_projection(&snapshot_projection(&started));
        let terminal = project_normalized_event(
            restored,
            5,
            &NormalizedProviderEvent::TurnEnded {
                turn_id: "turn-a".into(),
            },
        )
        .state;
        assert_eq!(terminal.lifecycle.root_phase, TurnPhase::Ready);
        assert_eq!(terminal.active_turn_id, None);
    }
}
