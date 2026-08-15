//! Pure runtime policy and reducer rules. This crate never opens a database or process.

use std::collections::BTreeMap;

use gent_types::{ConversationLiveStatus, HostEpoch, TurnPhase, WorkPhase};

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum CoreError {
    #[error("stale host epoch: command {command:?}, active {active:?}")]
    StaleEpoch {
        command: HostEpoch,
        active: HostEpoch,
    },
}

/// Rejects commands issued by a superseded writer.
///
/// # Errors
/// Returns [`CoreError::StaleEpoch`] when the command does not carry the active epoch.
pub fn require_current_epoch(command: HostEpoch, active: HostEpoch) -> Result<(), CoreError> {
    if command == active {
        Ok(())
    } else {
        Err(CoreError::StaleEpoch { command, active })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Run {
    pub id: String,
    pub parent_run_id: Option<String>,
    pub provider: String,
}

/// A provider switch deliberately creates a new immutable lineage node.
#[must_use]
pub fn switch_provider(run: &Run, child_id: String, provider: String) -> Run {
    Run {
        id: child_id,
        parent_run_id: Some(run.id.clone()),
        provider,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleState {
    pub root_phase: TurnPhase,
    pub children: BTreeMap<String, WorkPhase>,
    pub commands: BTreeMap<String, WorkPhase>,
    pub needs_attention: bool,
    pub has_error: bool,
}

impl Default for LifecycleState {
    fn default() -> Self {
        Self {
            root_phase: TurnPhase::Ready,
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
}

/// Pure lifecycle reducer: no persistence, clock, process, or presentation dependency.
#[must_use]
pub fn reduce_lifecycle(mut state: LifecycleState, event: LifecycleEvent) -> LifecycleState {
    match event {
        LifecycleEvent::RootPhase(phase) => state.root_phase = phase,
        LifecycleEvent::ChildPhase { child_id, phase } => {
            state.children.insert(child_id, phase);
        }
        LifecycleEvent::CommandPhase { command_id, phase } => {
            state.commands.insert(command_id, phase);
        }
        LifecycleEvent::AttentionRequired => state.needs_attention = true,
        LifecycleEvent::AttentionCleared => state.needs_attention = false,
    }
    state
}

#[must_use]
pub fn live_status(state: &LifecycleState, snapshot_cursor: u64) -> ConversationLiveStatus {
    let has_live_subagent_work = state.children.values().any(WorkPhase::is_live);
    let has_live_command_work = state.commands.values().any(WorkPhase::is_live);
    let is_processing = matches!(
        state.root_phase,
        TurnPhase::Processing | TurnPhase::Compacting
    );
    ConversationLiveStatus {
        snapshot_cursor,
        is_processing,
        is_waiting_for_subagents: !is_processing && has_live_subagent_work,
        has_live_subagent_work,
        is_waiting_for_command: !is_processing && has_live_command_work,
        has_live_command_work,
        needs_attention: state.needs_attention,
        has_error: state.has_error
            || matches!(state.root_phase, TurnPhase::Dead | TurnPhase::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CoreError, LifecycleEvent, LifecycleState, Run, live_status, reduce_lifecycle,
        require_current_epoch, switch_provider,
    };
    use gent_types::{HostEpoch, TurnPhase, WorkPhase};

    #[test]
    fn stale_epoch_is_rejected() {
        assert_eq!(
            require_current_epoch(HostEpoch(1), HostEpoch(2)),
            Err(CoreError::StaleEpoch {
                command: HostEpoch(1),
                active: HostEpoch(2)
            })
        );
    }

    #[test]
    fn provider_switch_preserves_parent() {
        let parent = Run {
            id: "run-a".into(),
            parent_run_id: None,
            provider: "claude".into(),
        };
        let child = switch_provider(&parent, "run-b".into(), "codex".into());
        assert_eq!(child.parent_run_id.as_deref(), Some("run-a"));
        assert_eq!(parent.provider, "claude");
    }

    #[test]
    fn terminal_root_does_not_hide_live_detached_subagent_work() {
        let state = reduce_lifecycle(
            LifecycleState::default(),
            LifecycleEvent::ChildPhase {
                child_id: "child".into(),
                phase: WorkPhase::Running,
            },
        );
        let state = reduce_lifecycle(state, LifecycleEvent::RootPhase(TurnPhase::Ready));
        let status = live_status(&state, 12);
        assert!(!status.is_processing);
        assert!(status.is_waiting_for_subagents);
        assert!(!status.is_waiting_for_command);
    }
}
