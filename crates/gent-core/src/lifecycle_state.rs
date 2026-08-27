//! Pure lifecycle reducer and status derivation.

use std::collections::BTreeMap;

use gent_types::{
    ConversationAttentionStatus, ConversationErrorStatus, ConversationLiveStatus,
    ConversationProcessingStatus, ConversationWorkStatus, RootActivity, TurnPhase, WorkPhase,
};

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
pub fn live_status(state: &LifecycleState, cursor: u64) -> ConversationLiveStatus {
    let subagent_work = work_status(state.children.values(), state.root_activity);
    let command_work = work_status(state.commands.values(), state.root_activity);
    // Adapters may report a generation fact before the corresponding turn-phase update arrives.
    // Keep a loading indicator truthful during that ordered, durable transition.
    let processing = state.root_activity.is_generating()
        || matches!(
            state.root_phase,
            TurnPhase::Processing
                | TurnPhase::WaitingPermission
                | TurnPhase::WaitingQuestion
                | TurnPhase::Compacting
        );
    ConversationLiveStatus {
        cursor,
        processing: if processing {
            ConversationProcessingStatus::Processing
        } else {
            ConversationProcessingStatus::default()
        },
        subagent_work,
        command_work,
        attention: if state.needs_attention {
            ConversationAttentionStatus::Required
        } else {
            ConversationAttentionStatus::default()
        },
        error: if state.has_error || matches!(state.root_phase, TurnPhase::Dead | TurnPhase::Failed)
        {
            ConversationErrorStatus::Error
        } else {
            ConversationErrorStatus::default()
        },
    }
}

fn work_status<'a>(
    mut phases: impl Iterator<Item = &'a WorkPhase>,
    root_activity: RootActivity,
) -> ConversationWorkStatus {
    if !phases.any(WorkPhase::is_live) {
        ConversationWorkStatus::None
    } else if root_activity.is_generating() {
        ConversationWorkStatus::Active
    } else {
        ConversationWorkStatus::Waiting
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
        assert!(live_status(&waiting, 1).is_processing());
        assert!(live_status(&waiting, 1).is_waiting_for_subagents());
        assert!(live_status(&waiting, 1).is_waiting_for_command());

        let generating = reduce_lifecycle(
            state,
            LifecycleEvent::RootActivity(RootActivity::Generating),
        );
        assert!(live_status(&generating, 1).is_processing());
        assert!(!live_status(&generating, 1).is_waiting_for_subagents());
        assert!(!live_status(&generating, 1).is_waiting_for_command());
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
        assert!(live_status(&state, 1).is_waiting_for_command());
    }

    #[test]
    fn explicit_generation_keeps_loading_true_before_a_phase_update() {
        let state = reduce_lifecycle(
            LifecycleState::default(),
            LifecycleEvent::RootActivity(RootActivity::Generating),
        );
        let status = live_status(&state, 1);
        assert!(status.is_processing());
        assert!(!status.is_waiting_for_subagents());
        assert!(!status.is_waiting_for_command());
    }
}
