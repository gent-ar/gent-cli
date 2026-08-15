//! Pure runtime policy and reducer rules. This crate never opens a database or process.
use std::collections::BTreeMap;

use gent_types::{ConversationLiveStatus, HostEpoch, TurnPhase, WorkPhase};

mod automation_execution;
mod decision_settlement;
mod git_operation;
mod lifecycle_projection;
mod lifecycle_signal;
mod projection_snapshot;
mod turn_lifecycle;
pub use automation_execution::permits_automation_execution_transition;
pub use decision_settlement::{
    DecisionCommandOutcome, DecisionCommandUpdate, DecisionEvidence, DecisionEvidenceUpdate,
    DecisionSettlementState, apply_decision_evidence, submit_decision,
};
pub use git_operation::permits_git_operation_transition;
pub use lifecycle_projection::{
    LifecycleProjection, ProjectionUpdate, project_normalized_event, projected_live_status,
};
pub use lifecycle_signal::project_lifecycle_signal;
pub use projection_snapshot::{restore_projection, snapshot_projection};
pub use turn_lifecycle::permits_turn_transition;

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum CoreError {
    #[error("stale host epoch: command {command:?}, active {active:?}")]
    StaleEpoch {
        command: HostEpoch,
        active: HostEpoch,
    },
    #[error("ingress is closed at epoch {epoch:?}")]
    IngressClosed { epoch: HostEpoch },
    #[error("lease epoch {lease:?} does not match active host epoch {active:?}")]
    LeaseEpoch { lease: HostEpoch, active: HostEpoch },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngressMode {
    Open,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IngressState {
    pub epoch: HostEpoch,
    pub mode: IngressMode,
}

/// Pure fence validation used by every mutating ingress adapter.
///
/// # Errors
/// Returns a stale-epoch or closed-ingress error when mutation is not permitted.
pub fn validate_ingress(command: HostEpoch, state: IngressState) -> Result<(), CoreError> {
    if command != state.epoch {
        return Err(CoreError::StaleEpoch {
            command,
            active: state.epoch,
        });
    }
    if state.mode == IngressMode::Closed {
        return Err(CoreError::IngressClosed { epoch: state.epoch });
    }
    Ok(())
}

/// Returns the immutable successor epoch used by a new writer.
#[must_use]
pub const fn next_epoch(epoch: HostEpoch) -> HostEpoch {
    HostEpoch(epoch.0.saturating_add(1))
}

/// Rejects commands issued by a superseded writer.
///
/// # Errors
/// Returns [`CoreError::StaleEpoch`] when the command does not carry the active epoch.
pub fn require_current_epoch(command: HostEpoch, active: HostEpoch) -> Result<(), CoreError> {
    validate_ingress(
        command,
        IngressState {
            epoch: active,
            mode: IngressMode::Open,
        },
    )
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
pub struct Lease {
    pub worktree_id: String,
    pub run_id: String,
    pub token: String,
    pub epoch: HostEpoch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaseResolution {
    Acquire,
    Contended(Lease),
    Recover(Lease),
}

/// Decides lease ownership without inspecting a database or clock.
///
/// # Errors
/// Returns an error when the requesting run was not created by the active host.
pub fn resolve_lease(
    existing: Option<&Lease>,
    requested: &Lease,
    active: HostEpoch,
) -> Result<LeaseResolution, CoreError> {
    if requested.epoch != active {
        return Err(CoreError::LeaseEpoch {
            lease: requested.epoch,
            active,
        });
    }
    match existing {
        None => Ok(LeaseResolution::Acquire),
        Some(lease) if lease.epoch == active => Ok(LeaseResolution::Contended(lease.clone())),
        Some(lease) => Ok(LeaseResolution::Recover(lease.clone())),
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
    ErrorRaised,
    ErrorCleared,
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
    use super::*;

    #[test]
    fn ingress_rejects_stale_and_closed_commands() {
        assert!(matches!(
            validate_ingress(
                HostEpoch(1),
                IngressState {
                    epoch: HostEpoch(2),
                    mode: IngressMode::Open
                }
            ),
            Err(CoreError::StaleEpoch { .. })
        ));
        assert_eq!(
            validate_ingress(
                HostEpoch(2),
                IngressState {
                    epoch: HostEpoch(2),
                    mode: IngressMode::Closed
                }
            ),
            Err(CoreError::IngressClosed {
                epoch: HostEpoch(2)
            })
        );
    }

    #[test]
    fn stale_leases_recover_only_after_a_fence() {
        let lease = Lease {
            worktree_id: "tree".into(),
            run_id: "old".into(),
            token: "a".into(),
            epoch: HostEpoch(1),
        };
        let requested = Lease {
            worktree_id: "tree".into(),
            run_id: "new".into(),
            token: "b".into(),
            epoch: HostEpoch(2),
        };
        assert_eq!(
            resolve_lease(Some(&lease), &requested, HostEpoch(2)),
            Ok(LeaseResolution::Recover(lease))
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
        let status = live_status(
            &reduce_lifecycle(state, LifecycleEvent::RootPhase(TurnPhase::Ready)),
            12,
        );
        assert!(status.is_waiting_for_subagents);
    }
}
