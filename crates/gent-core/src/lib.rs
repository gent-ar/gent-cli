//! Pure runtime policy and reducer rules. This crate never opens a database or process.
use gent_types::HostEpoch;
mod attachment_transfer;
mod conversation_activity;
mod decision_settlement;
mod git_operation;
mod goal;
#[cfg(test)]
mod goal_tests;
mod lifecycle_projection;
mod lifecycle_signal;
mod lifecycle_state;
mod observer_comparison;
mod permission_control;
#[cfg(test)]
mod permission_control_tests;
mod permission_policy;
mod projection_snapshot;
mod provider_auth;
mod reviewed_plan;
mod runtime_update;
mod runtime_update_schedule;
mod tool_classification;
mod turn_lifecycle;
pub use attachment_transfer::*;
pub use conversation_activity::{
    ConversationActivityProjection, ConversationActivityUpdate, project_conversation_activity,
};
pub use decision_settlement::{
    DecisionCommandOutcome, DecisionCommandUpdate, DecisionEvidence, DecisionEvidenceUpdate,
    DecisionSettlementState, apply_decision_evidence, submit_decision,
};
pub use git_operation::permits_git_operation_transition;
pub use goal::{
    ActiveGoalRejection, ActiveGoalSelection, GoalControlContext, GoalControlEffect,
    GoalControlEvent, GoalControlRejection, GoalControlState, reduce_goal_control,
    select_active_goal,
};
pub use lifecycle_projection::{
    LifecycleProjection, ProjectionUpdate, project_normalized_event, projected_live_status,
};
pub use lifecycle_signal::project_lifecycle_signal;
pub use lifecycle_state::{LifecycleEvent, LifecycleState, live_status, reduce_lifecycle};
pub use observer_comparison::{ObserverComparison, ObserverProjection, compare_legacy_tap};
pub use permission_control::{
    PermissionControlContext, PermissionControlEffect, PermissionControlEvent,
    PermissionControlRejection, PermissionControlResolution, PermissionControlState,
    reduce_permission_control,
};
pub use permission_policy::{
    PermissionDecision, evaluate_permission, evaluate_permission_with_sandbox,
};
pub use projection_snapshot::{restore_projection, snapshot_projection};
pub use provider_auth::{
    ProviderAuthEffect, ProviderAuthEvent, ProviderAuthRejection, ProviderAuthState,
    reduce_provider_auth,
};
pub use reviewed_plan::{
    ReviewedPlanEffect, ReviewedPlanEvent, ReviewedPlanRejection, ReviewedPlanState,
    reduce_reviewed_plan,
};
pub use runtime_update::{
    RuntimeUpdateContext, RuntimeUpdateEligibility, RuntimeUpdateEvent, RuntimeUpdateIngress,
    RuntimeUpdateTransition, assess_runtime_update, reduce_runtime_update,
};
pub use runtime_update_schedule::{
    RuntimeUpdateCheckOutcome, RuntimeUpdateSchedule, RuntimeUpdateScheduleDecision,
    RuntimeUpdateScheduleState, record_runtime_update_check, schedule_runtime_update_check,
};
pub use tool_classification::ToolCatalog;
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
}
