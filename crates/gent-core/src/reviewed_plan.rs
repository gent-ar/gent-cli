//! Pure reviewed-plan approval rules with an explicit child-run context boundary.

use gent_types::{
    ContextPolicy, PlanArtifact, PlanRevision, PlanStatus, ReviewedPlanId,
    StartImplementationRequest,
};

/// Pure state retained for one durable reviewed-plan projection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReviewedPlanState {
    pub plan: Option<PlanArtifact>,
    pub approval: Option<StartImplementationRequest>,
}

/// Facts and user choices accepted by the reviewed-plan reducer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewedPlanEvent {
    Observed(PlanArtifact),
    Approve {
        request: StartImplementationRequest,
        current_policy_revision: u64,
        history_through_ordinal: u64,
    },
    Reject {
        plan_id: ReviewedPlanId,
        revision: PlanRevision,
        content_digest_sha256: String,
    },
    ImplementationFailed,
}

/// The one authority-side action a future runtime may perform after approval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewedPlanEffect {
    None,
    ReserveImplementation {
        request: Box<StartImplementationRequest>,
        context_through_ordinal: u64,
    },
    Rejected(ReviewedPlanRejection),
}

/// Rejections exclude all reviewed-plan content and provider data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewedPlanRejection {
    InvalidPlan,
    InvalidApproval,
    NoPlan,
    PlanMismatch,
    PlanNotReviewable,
    PolicyRevisionMismatch,
    AlreadyApproved,
}

/// Reduces a reviewed-plan fact or approval without I/O, storage, or provider access.
#[must_use]
pub fn reduce_reviewed_plan(
    state: ReviewedPlanState,
    event: ReviewedPlanEvent,
) -> (ReviewedPlanState, ReviewedPlanEffect) {
    match event {
        ReviewedPlanEvent::Observed(plan) => observe(state, plan),
        ReviewedPlanEvent::Approve {
            request,
            current_policy_revision,
            history_through_ordinal,
        } => approve(
            state,
            request,
            current_policy_revision,
            history_through_ordinal,
        ),
        ReviewedPlanEvent::Reject {
            plan_id,
            revision,
            content_digest_sha256,
        } => reject(state, &plan_id, revision, &content_digest_sha256),
        ReviewedPlanEvent::ImplementationFailed => failed(state),
    }
}

fn observe(
    mut state: ReviewedPlanState,
    plan: PlanArtifact,
) -> (ReviewedPlanState, ReviewedPlanEffect) {
    if plan.validate().is_err() {
        return (
            state,
            ReviewedPlanEffect::Rejected(ReviewedPlanRejection::InvalidPlan),
        );
    }
    let Some(current) = &state.plan else {
        state.plan = Some(plan);
        return (state, ReviewedPlanEffect::None);
    };
    if current.plan_id != plan.plan_id || plan.revision < current.revision {
        return (state, ReviewedPlanEffect::None);
    }
    if plan.revision == current.revision
        && plan.content_digest_sha256 != current.content_digest_sha256
    {
        return (
            state,
            ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PlanMismatch),
        );
    }
    if plan.revision > current.revision {
        state.approval = None;
    }
    state.plan = Some(plan);
    (state, ReviewedPlanEffect::None)
}

fn approve(
    mut state: ReviewedPlanState,
    request: StartImplementationRequest,
    current_policy_revision: u64,
    history_through_ordinal: u64,
) -> (ReviewedPlanState, ReviewedPlanEffect) {
    if request.validate().is_err() {
        return (
            state,
            ReviewedPlanEffect::Rejected(ReviewedPlanRejection::InvalidApproval),
        );
    }
    let Some(plan) = state.plan.as_mut() else {
        return (
            state,
            ReviewedPlanEffect::Rejected(ReviewedPlanRejection::NoPlan),
        );
    };
    if !matches_plan(plan, &request) {
        return (
            state,
            ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PlanMismatch),
        );
    }
    if request.policy_revision != current_policy_revision {
        return (
            state,
            ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PolicyRevisionMismatch),
        );
    }
    if let Some(existing) = &state.approval {
        return if existing == &request {
            (state, ReviewedPlanEffect::None)
        } else {
            (
                state,
                ReviewedPlanEffect::Rejected(ReviewedPlanRejection::AlreadyApproved),
            )
        };
    }
    if plan.status != PlanStatus::ReadyForReview {
        return (
            state,
            ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PlanNotReviewable),
        );
    }
    plan.status = PlanStatus::Approved;
    state.approval = Some(request.clone());
    let context_through_ordinal = match request.context_policy {
        ContextPolicy::Preserve => history_through_ordinal,
        ContextPolicy::Clear => 0,
    };
    (
        state,
        ReviewedPlanEffect::ReserveImplementation {
            request: Box::new(request),
            context_through_ordinal,
        },
    )
}

fn reject(
    mut state: ReviewedPlanState,
    plan_id: &ReviewedPlanId,
    revision: PlanRevision,
    content_digest_sha256: &str,
) -> (ReviewedPlanState, ReviewedPlanEffect) {
    let Some(plan) = state.plan.as_mut() else {
        return (
            state,
            ReviewedPlanEffect::Rejected(ReviewedPlanRejection::NoPlan),
        );
    };
    if &plan.plan_id != plan_id
        || plan.revision != revision
        || plan.content_digest_sha256 != content_digest_sha256
    {
        return (
            state,
            ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PlanMismatch),
        );
    }
    if plan.status != PlanStatus::ReadyForReview {
        return (
            state,
            ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PlanNotReviewable),
        );
    }
    plan.status = PlanStatus::Rejected;
    (state, ReviewedPlanEffect::None)
}

fn failed(mut state: ReviewedPlanState) -> (ReviewedPlanState, ReviewedPlanEffect) {
    if let Some(plan) = &mut state.plan {
        plan.status = PlanStatus::TerminallyFailed;
    }
    (state, ReviewedPlanEffect::None)
}

fn matches_plan(plan: &PlanArtifact, request: &StartImplementationRequest) -> bool {
    plan.plan_id == request.plan_id
        && plan.conversation_id == request.conversation_id
        && plan.source_run_id == request.parent_run_id
        && plan.revision == request.plan_revision
        && plan.content_digest_sha256 == request.plan_content_digest_sha256
}

#[cfg(test)]
#[path = "reviewed_plan_reducer_tests.rs"]
mod reducer_tests;
#[cfg(test)]
#[path = "reviewed_plan_tests.rs"]
mod tests;
