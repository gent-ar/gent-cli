use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, ContextPolicy, HostEpoch, PermissionCategory, PlanAction,
    PlanActionKind, PlanArtifact, PlanPermissionPreview, PlanRevision, PlanStatus, ReceiptId,
    ReviewedPlanId, StartImplementationRequest,
};

use super::{
    ReviewedPlanEffect, ReviewedPlanEvent, ReviewedPlanRejection, ReviewedPlanState,
    reduce_reviewed_plan,
};

fn plan() -> PlanArtifact {
    PlanArtifact {
        plan_id: ReviewedPlanId("plan-1".into()),
        conversation_id: AgentChatConversationId("conversation-1".into()),
        source_run_id: AgentChatRunId("run-1".into()),
        source_turn_id: "turn-1".into(),
        revision: PlanRevision(1),
        content_digest_sha256: "a".repeat(64),
        status: PlanStatus::ReadyForReview,
        actions: vec![PlanAction {
            action_id: "action-1".into(),
            kind: PlanActionKind::Edit,
            summary: "Update one file".into(),
        }],
        risks: Vec::new(),
        diffs: Vec::new(),
        permission_preview: vec![PlanPermissionPreview {
            category: PermissionCategory::Edit,
            summary: "Modify one file".into(),
        }],
    }
}

fn approval(policy: ContextPolicy) -> StartImplementationRequest {
    StartImplementationRequest {
        request_id: AgentChatRequestId("request-1".into()),
        receipt_id: ReceiptId("receipt-1".into()),
        idempotency_key: "key-1".into(),
        host_epoch: HostEpoch(1),
        policy_workspace_id: "workspace-1".into(),
        policy_revision: 2,
        conversation_id: AgentChatConversationId("conversation-1".into()),
        plan_id: ReviewedPlanId("plan-1".into()),
        plan_revision: PlanRevision(1),
        plan_content_digest_sha256: "a".repeat(64),
        parent_run_id: AgentChatRunId("run-1".into()),
        selection: AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Agent,
        },
        context_policy: policy,
    }
}

fn reviewed_state() -> ReviewedPlanState {
    reduce_reviewed_plan(
        ReviewedPlanState::default(),
        ReviewedPlanEvent::Observed(plan()),
    )
    .0
}

#[test]
fn approval_binds_selection_policy_revision_and_preserved_history() {
    let request = approval(ContextPolicy::Preserve);
    let (state, effect) = reduce_reviewed_plan(
        reviewed_state(),
        ReviewedPlanEvent::Approve {
            request: request.clone(),
            current_policy_revision: 2,
            history_through_ordinal: 9,
        },
    );
    assert!(matches!(
        effect,
        ReviewedPlanEffect::ReserveImplementation {
            request: value,
            context_through_ordinal: 9,
        } if *value == request
    ));
    assert_eq!(state.plan.unwrap().status, PlanStatus::Approved);
}

#[test]
fn clear_context_creates_a_fresh_child_boundary_without_deleting_the_plan() {
    let (state, effect) = reduce_reviewed_plan(
        reviewed_state(),
        ReviewedPlanEvent::Approve {
            request: approval(ContextPolicy::Clear),
            current_policy_revision: 2,
            history_through_ordinal: 9,
        },
    );
    assert!(matches!(
        effect,
        ReviewedPlanEffect::ReserveImplementation {
            context_through_ordinal: 0,
            ..
        }
    ));
    assert_eq!(state.plan.unwrap().plan_id.0, "plan-1");
}

#[test]
fn policy_drift_and_duplicate_approval_are_safe() {
    let request = approval(ContextPolicy::Preserve);
    let (_, drift) = reduce_reviewed_plan(
        reviewed_state(),
        ReviewedPlanEvent::Approve {
            request: request.clone(),
            current_policy_revision: 3,
            history_through_ordinal: 1,
        },
    );
    assert_eq!(
        drift,
        ReviewedPlanEffect::Rejected(ReviewedPlanRejection::PolicyRevisionMismatch)
    );
    let (state, _) = reduce_reviewed_plan(
        reviewed_state(),
        ReviewedPlanEvent::Approve {
            request: request.clone(),
            current_policy_revision: 2,
            history_through_ordinal: 1,
        },
    );
    assert_eq!(
        reduce_reviewed_plan(
            state,
            ReviewedPlanEvent::Approve {
                request,
                current_policy_revision: 2,
                history_through_ordinal: 1,
            },
        )
        .1,
        ReviewedPlanEffect::None
    );
}
