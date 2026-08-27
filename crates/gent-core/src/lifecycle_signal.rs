//! Pure projection of additive lifecycle signals from a provider adapter.

use gent_types::NormalizedLifecycleSignal;

use crate::{LifecycleEvent, LifecycleProjection, ProjectionUpdate, reduce_lifecycle};

/// Applies a strictly newer lifecycle signal cursor to a run projection.
#[must_use]
pub fn project_lifecycle_signal(
    mut state: LifecycleProjection,
    cursor: u64,
    signal: &NormalizedLifecycleSignal,
) -> ProjectionUpdate {
    if state.last_cursor.is_some_and(|last| cursor <= last) {
        return ProjectionUpdate {
            state,
            applied: false,
        };
    }
    if let Some(event) = lifecycle_event(signal) {
        state.lifecycle = reduce_lifecycle(state.lifecycle, event);
    }
    state.last_cursor = Some(cursor);
    ProjectionUpdate {
        state,
        applied: true,
    }
}

fn lifecycle_event(signal: &NormalizedLifecycleSignal) -> Option<LifecycleEvent> {
    Some(match signal {
        NormalizedLifecycleSignal::RootPhase { phase } => LifecycleEvent::RootPhase(phase.clone()),
        NormalizedLifecycleSignal::RootActivity { activity } => {
            LifecycleEvent::RootActivity(*activity)
        }
        NormalizedLifecycleSignal::ChildPhase { child_id, phase } => LifecycleEvent::ChildPhase {
            child_id: child_id.clone(),
            phase: phase.clone(),
        },
        NormalizedLifecycleSignal::CommandPhase { command_id, phase } => {
            LifecycleEvent::CommandPhase {
                command_id: command_id.clone(),
                phase: phase.clone(),
            }
        }
        NormalizedLifecycleSignal::ToolActivity { .. } => return None,
        NormalizedLifecycleSignal::AttentionRequired => LifecycleEvent::AttentionRequired,
        NormalizedLifecycleSignal::AttentionCleared => LifecycleEvent::AttentionCleared,
    })
}

#[cfg(test)]
mod tests {
    use gent_types::{
        NormalizedLifecycleSignal, RootActivity, ToolActivity, ToolPhase, TurnPhase, WorkPhase,
    };

    use super::project_lifecycle_signal;
    use crate::{LifecycleProjection, projected_live_status};

    #[test]
    fn waiting_and_attention_signals_project_without_provider_content() {
        let waiting = project_lifecycle_signal(
            LifecycleProjection::default(),
            1,
            &NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::WaitingQuestion,
            },
        )
        .state;
        assert!(projected_live_status(&waiting).is_processing());
        let command = project_lifecycle_signal(
            waiting,
            2,
            &NormalizedLifecycleSignal::CommandPhase {
                command_id: "command-1".into(),
                phase: WorkPhase::WaitingPermission,
            },
        )
        .state;
        assert!(projected_live_status(&command).has_live_command_work());
        let waiting = project_lifecycle_signal(
            command,
            3,
            &NormalizedLifecycleSignal::RootActivity {
                activity: RootActivity::Waiting,
            },
        )
        .state;
        assert!(projected_live_status(&waiting).is_waiting_for_command());
        let attention =
            project_lifecycle_signal(waiting, 4, &NormalizedLifecycleSignal::AttentionRequired)
                .state;
        assert!(projected_live_status(&attention).needs_attention());
    }

    #[test]
    fn every_work_signal_is_reduced_and_stale_cursor_is_ignored() {
        let child = project_lifecycle_signal(
            LifecycleProjection::default(),
            3,
            &NormalizedLifecycleSignal::ChildPhase {
                child_id: "child".into(),
                phase: WorkPhase::Running,
            },
        )
        .state;
        let command = project_lifecycle_signal(
            child,
            4,
            &NormalizedLifecycleSignal::CommandPhase {
                command_id: "command".into(),
                phase: WorkPhase::Done,
            },
        )
        .state;
        let cleared = project_lifecycle_signal(
            command.clone(),
            5,
            &NormalizedLifecycleSignal::AttentionCleared,
        );
        assert!(cleared.applied);
        assert!(cleared.state.lifecycle.children.contains_key("child"));
        assert!(cleared.state.lifecycle.commands.contains_key("command"));
        let stale =
            project_lifecycle_signal(command, 4, &NormalizedLifecycleSignal::AttentionRequired);
        assert!(!stale.applied);
        assert!(!projected_live_status(&stale.state).needs_attention());
    }

    #[test]
    fn tool_activity_advances_the_durable_cursor_without_changing_lifecycle_state() {
        let update = project_lifecycle_signal(
            LifecycleProjection::default(),
            1,
            &NormalizedLifecycleSignal::ToolActivity {
                activity: ToolActivity {
                    tool_use_id: "tool-1".into(),
                    tool_name: "read_file".into(),
                    phase: ToolPhase::Started,
                    output_digest: None,
                },
            },
        );
        assert!(update.applied);
        assert_eq!(update.state.last_cursor, Some(1));
        assert!(!projected_live_status(&update.state).is_processing());
    }
}
