use gent_types::{AgentChatRunId, ContextPolicy, PlanRevision, PlanStatus, ReviewedPlanId};

use super::tests::{approval, plan, reviewed_state};
use super::{
    ReviewedPlanEffect, ReviewedPlanEvent, ReviewedPlanRejection, ReviewedPlanState,
    reduce_reviewed_plan,
};

#[test]
fn observation_rejects_invalid_conflicting_and_stale_revisions() {
    let mut invalid = plan();
    invalid.content_digest_sha256.clear();
    assert_eq!(
        reduce_reviewed_plan(
            ReviewedPlanState::default(),
            ReviewedPlanEvent::Observed(invalid)
        )
        .1,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::InvalidPlan)
    );
    let state = reviewed_state();
    let mut other = plan();
    other.plan_id = ReviewedPlanId("plan-2".into());
    assert_eq!(
        reduce_reviewed_plan(state.clone(), ReviewedPlanEvent::Observed(other)).0,
        state
    );
    let mut zero_revision = plan();
    zero_revision.revision = PlanRevision(0);
    assert_eq!(
        reduce_reviewed_plan(state.clone(), ReviewedPlanEvent::Observed(zero_revision)).0,
        state
    );
    let mut conflicting = plan();
    conflicting.content_digest_sha256 = "b".repeat(64);
    assert_eq!(
        reduce_reviewed_plan(state, ReviewedPlanEvent::Observed(conflicting)).1,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PlanMismatch)
    );
}

#[test]
fn newer_observation_replaces_plan_and_clears_an_existing_approval() {
    let (approved, _) = reduce_reviewed_plan(
        reviewed_state(),
        ReviewedPlanEvent::Approve {
            request: approval(ContextPolicy::Preserve),
            current_policy_revision: 2,
            history_through_ordinal: 1,
        },
    );
    let mut newer = plan();
    newer.revision = PlanRevision(2);
    newer.content_digest_sha256 = "b".repeat(64);
    let (state, effect) = reduce_reviewed_plan(approved, ReviewedPlanEvent::Observed(newer));
    assert_eq!(effect, ReviewedPlanEffect::None);
    assert_eq!(state.plan.unwrap().revision, PlanRevision(2));
    assert_eq!(state.approval, None);
}

#[test]
fn approval_rejects_invalid_missing_mismatched_and_non_reviewable_plans() {
    let mut invalid = approval(ContextPolicy::Preserve);
    invalid.idempotency_key.clear();
    assert_eq!(
        reduce_reviewed_plan(
            reviewed_state(),
            ReviewedPlanEvent::Approve {
                request: invalid,
                current_policy_revision: 2,
                history_through_ordinal: 1,
            },
        )
        .1,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::InvalidApproval)
    );
    assert_eq!(
        reduce_reviewed_plan(
            ReviewedPlanState::default(),
            ReviewedPlanEvent::Approve {
                request: approval(ContextPolicy::Preserve),
                current_policy_revision: 2,
                history_through_ordinal: 1,
            },
        )
        .1,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::NoPlan)
    );
    let mut mismatched = approval(ContextPolicy::Preserve);
    mismatched.parent_run_id = AgentChatRunId("run-2".into());
    assert_eq!(
        reduce_reviewed_plan(
            reviewed_state(),
            ReviewedPlanEvent::Approve {
                request: mismatched,
                current_policy_revision: 2,
                history_through_ordinal: 1,
            },
        )
        .1,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PlanMismatch)
    );
    let mut draft = plan();
    draft.status = PlanStatus::Draft;
    let (state, _) = reduce_reviewed_plan(
        ReviewedPlanState::default(),
        ReviewedPlanEvent::Observed(draft),
    );
    assert_eq!(
        reduce_reviewed_plan(
            state,
            ReviewedPlanEvent::Approve {
                request: approval(ContextPolicy::Preserve),
                current_policy_revision: 2,
                history_through_ordinal: 1,
            },
        )
        .1,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PlanNotReviewable)
    );
}

#[test]
fn approval_rejects_a_different_request_after_reservation() {
    let (state, _) = reduce_reviewed_plan(
        reviewed_state(),
        ReviewedPlanEvent::Approve {
            request: approval(ContextPolicy::Preserve),
            current_policy_revision: 2,
            history_through_ordinal: 1,
        },
    );
    let mut different = approval(ContextPolicy::Preserve);
    different.idempotency_key = "key-2".into();
    assert_eq!(
        reduce_reviewed_plan(
            state,
            ReviewedPlanEvent::Approve {
                request: different,
                current_policy_revision: 2,
                history_through_ordinal: 1,
            },
        )
        .1,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::AlreadyApproved)
    );
}

#[test]
fn rejection_and_failure_only_change_the_exact_reviewed_plan() {
    let mismatch = ReviewedPlanEvent::Reject {
        plan_id: ReviewedPlanId("plan-2".into()),
        revision: PlanRevision(1),
        content_digest_sha256: "a".repeat(64),
    };
    assert_eq!(
        reduce_reviewed_plan(reviewed_state(), mismatch).1,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PlanMismatch)
    );
    assert_eq!(
        reduce_reviewed_plan(
            ReviewedPlanState::default(),
            ReviewedPlanEvent::Reject {
                plan_id: ReviewedPlanId("plan-1".into()),
                revision: PlanRevision(1),
                content_digest_sha256: "a".repeat(64),
            },
        )
        .1,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::NoPlan)
    );
    let (rejected, effect) = reduce_reviewed_plan(
        reviewed_state(),
        ReviewedPlanEvent::Reject {
            plan_id: ReviewedPlanId("plan-1".into()),
            revision: PlanRevision(1),
            content_digest_sha256: "a".repeat(64),
        },
    );
    assert_eq!(effect, ReviewedPlanEffect::None);
    assert_eq!(rejected.plan.as_ref().unwrap().status, PlanStatus::Rejected);
    assert_eq!(
        reduce_reviewed_plan(
            rejected,
            ReviewedPlanEvent::Reject {
                plan_id: ReviewedPlanId("plan-1".into()),
                revision: PlanRevision(1),
                content_digest_sha256: "a".repeat(64),
            },
        )
        .1,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PlanNotReviewable)
    );
    let (failed_empty, _) = reduce_reviewed_plan(
        ReviewedPlanState::default(),
        ReviewedPlanEvent::ImplementationFailed,
    );
    assert_eq!(failed_empty, ReviewedPlanState::default());
    let (failed, _) =
        reduce_reviewed_plan(reviewed_state(), ReviewedPlanEvent::ImplementationFailed);
    assert_eq!(failed.plan.unwrap().status, PlanStatus::TerminallyFailed);
}
