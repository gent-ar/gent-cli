//! Pure lifecycle reducer and status derivation.

use std::collections::BTreeMap;

use gent_types::{ConversationLiveStatus, RootActivity, TurnPhase, WorkPhase};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleState {
    pub root_phase: TurnPhase,
    pub root_activity: RootActivity,
    pub children: BTreeMap<String, WorkPhase>,
    pub commands: BTreeMap<String, WorkPhase>,
    pub needs_attention: bool,
    pub has_error: bool,
}

impl Default for LifecycleState {
    fn default() -> Self {
        Self {
            root_phase: TurnPhase::Ready,
            root_activity: RootActivity::Idle,
            children: BTreeMap::new(),
            commands: BTreeMap::new(),
            needs_attention: false,
            has_error: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LifecycleEvent {
    RootPhase(TurnPhase),
    RootActivity(RootActivity),
    ChildPhase {
        child_id: String,
        phase: WorkPhase,
    },
    CommandPhase {
        command_id: String,
        phase: WorkPhase,
    },
    AttentionRequired,
    AttentionCleared,
    ErrorRaised,
    ErrorCleared,
}

/// Pure lifecycle reducer: no persistence, clock, process, or presentation dependency.
#[must_use]
pub fn reduce_lifecycle(mut state: LifecycleState, event: LifecycleEvent) -> LifecycleState {
    match event {
        LifecycleEvent::RootPhase(phase) => state.root_phase = phase,
        LifecycleEvent::RootActivity(activity) => state.root_activity = activity,
        LifecycleEvent::ChildPhase { child_id, phase } => {
            state.children.insert(child_id, phase);
        }
        LifecycleEvent::CommandPhase { command_id, phase } => {
            state.commands.insert(command_id, phase);
        }
        LifecycleEvent::AttentionRequired => state.needs_attention = true,
        LifecycleEvent::AttentionCleared => state.needs_attention = false,
        LifecycleEvent::ErrorRaised => state.has_error = true,
        LifecycleEvent::ErrorCleared => state.has_error = false,
    }
    state
}

#[must_use]
pub fn live_status(state: &LifecycleState, snapshot_cursor: u64) -> ConversationLiveStatus {
    let has_live_subagent_work = state.children.values().any(WorkPhase::is_live);
    let has_live_command_work = state.commands.values().any(WorkPhase::is_live);
    let is_processing = matches!(
        state.root_phase,
        TurnPhase::Processing
            | TurnPhase::WaitingPermission
            | TurnPhase::WaitingQuestion
            | TurnPhase::Compacting
    );
    ConversationLiveStatus {
        snapshot_cursor,
        is_processing,
        is_waiting_for_subagents: !state.root_activity.is_generating() && has_live_subagent_work,
        has_live_subagent_work,
        is_waiting_for_command: !state.root_activity.is_generating() && has_live_command_work,
        has_live_command_work,
        needs_attention: state.needs_attention,
        has_error: state.has_error
            || matches!(state.root_phase, TurnPhase::Dead | TurnPhase::Failed),
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{RootActivity, TurnPhase, WorkPhase};

    use super::{LifecycleEvent, LifecycleState, live_status, reduce_lifecycle};

    #[test]
    fn waiting_work_depends_on_explicit_root_activity_not_turn_phase() {
        let state = reduce_lifecycle(
            LifecycleState::default(),
            LifecycleEvent::RootPhase(TurnPhase::Processing),
        );
        let state = reduce_lifecycle(
            state,
            LifecycleEvent::ChildPhase {
                child_id: "child".into(),
                phase: WorkPhase::Running,
            },
        );
        let state = reduce_lifecycle(
            state,
            LifecycleEvent::CommandPhase {
                command_id: "command".into(),
                phase: WorkPhase::Running,
            },
        );
        let waiting = reduce_lifecycle(
            state.clone(),
            LifecycleEvent::RootActivity(RootActivity::Waiting),
        );
        assert!(live_status(&waiting, 1).is_processing);
        assert!(live_status(&waiting, 1).is_waiting_for_subagents);
        assert!(live_status(&waiting, 1).is_waiting_for_command);

        let generating = reduce_lifecycle(
            state,
            LifecycleEvent::RootActivity(RootActivity::Generating),
        );
        assert!(live_status(&generating, 1).is_processing);
        assert!(!live_status(&generating, 1).is_waiting_for_subagents);
        assert!(!live_status(&generating, 1).is_waiting_for_command);
    }

    #[test]
    fn idle_root_reports_live_command_work_as_waiting() {
        let state = reduce_lifecycle(
            LifecycleState::default(),
            LifecycleEvent::CommandPhase {
                command_id: "command".into(),
                phase: WorkPhase::Running,
            },
        );
        assert!(live_status(&state, 1).is_waiting_for_command);
    }
}
