use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
    AgentChatSelection, CrossReviewRequest, FanoutRequest, HarnessProfileRef, HostEpoch,
    ReviewCandidate, TaskGraph, TaskGraphBinding, TaskNode, TaskNodeSpec, TaskNodeStatus, TaskRole,
    WorktreePolicy,
};

use crate::{
    OrchestrationEffect, OrchestrationEvent, OrchestrationRejection, OrchestrationState,
    reduce_orchestration,
};

fn graph() -> TaskGraph {
    TaskGraph {
        binding: TaskGraphBinding {
            graph_id: "graph-1".into(),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            root_run_id: AgentChatRunId("run-1".into()),
            goal_id: "goal-1".into(),
            goal_revision: 1,
            policy_id: "policy-1".into(),
            policy_revision: 1,
            workspace_id: "workspace-1".into(),
            repository_id: "repo-1".into(),
            base_revision_digest_sha256: "b".repeat(64),
        },
        revision: 1,
        host_epoch: HostEpoch(1),
        idempotency_key: "fanout-1".into(),
        nodes: vec![TaskNode {
            spec: spec("candidate", TaskRole::Implementer, AgentChatProvider::Codex),
            revision: 2,
            status: TaskNodeStatus::Completed,
            result_artifact_digest: Some("a".repeat(64)),
        }],
    }
}
fn spec(id: &str, role: TaskRole, provider: AgentChatProvider) -> TaskNodeSpec {
    TaskNodeSpec {
        node_id: id.into(),
        role,
        profile: HarnessProfileRef {
            profile_id: format!("{id}-profile"),
            revision: 1,
            provider,
        },
        selection: AgentChatSelection {
            provider,
            model: "test".into(),
            effort: AgentChatEffort::Medium,
            mode: AgentChatMode::Agent,
        },
        input_artifact_digests: vec![],
        depends_on: vec![],
        worktree: WorktreePolicy::Isolated,
        retry_budget: 1,
    }
}
fn review() -> CrossReviewRequest {
    CrossReviewRequest {
        graph_id: "graph-1".into(),
        expected_graph_revision: 1,
        expected_parent_run_id: AgentChatRunId("run-1".into()),
        host_epoch: HostEpoch(1),
        goal_revision: 1,
        policy_revision: 1,
        idempotency_key: "review-1".into(),
        candidate: ReviewCandidate {
            node_id: "candidate".into(),
            node_revision: 2,
            artifact_digest_sha256: "a".repeat(64),
            base_revision_digest_sha256: "b".repeat(64),
        },
        reviewer: spec("reviewer", TaskRole::Reviewer, AgentChatProvider::Claude),
    }
}

#[test]
fn cross_review_requires_exact_fences_and_a_different_reviewer_provider() {
    let (state, _) = reduce_orchestration(
        OrchestrationState::default(),
        OrchestrationEvent::Fanout(FanoutRequest {
            graph: graph(),
            expected_parent_run_id: AgentChatRunId("run-1".into()),
        }),
    );
    let (state, effect) = reduce_orchestration(state, OrchestrationEvent::CrossReview(review()));
    assert!(matches!(effect, OrchestrationEffect::Persist(next) if next.nodes.len() == 2));
    let mut stale_request = review();
    stale_request.idempotency_key = "review-2".into();
    stale_request.expected_graph_revision = 1;
    assert_eq!(
        reduce_orchestration(state, OrchestrationEvent::CrossReview(stale_request)).1,
        OrchestrationEffect::Rejected(OrchestrationRejection::StaleGraphRevision)
    );
    let mut same_provider = review();
    same_provider.reviewer.profile.provider = AgentChatProvider::Codex;
    same_provider.reviewer.selection.provider = AgentChatProvider::Codex;
    assert_eq!(
        reduce_orchestration(
            OrchestrationState::new(Some(graph())),
            OrchestrationEvent::CrossReview(same_provider)
        )
        .1,
        OrchestrationEffect::Rejected(OrchestrationRejection::ReviewerMustCrossProvider)
    );
}

#[test]
fn rejected_commands_do_not_poison_their_idempotency_key() {
    let request = review();
    let (state, first) = reduce_orchestration(
        OrchestrationState::default(),
        OrchestrationEvent::CrossReview(request.clone()),
    );
    assert_eq!(
        first,
        OrchestrationEffect::Rejected(OrchestrationRejection::GraphMissing)
    );
    assert_eq!(
        reduce_orchestration(state, OrchestrationEvent::CrossReview(request)).1,
        OrchestrationEffect::Rejected(OrchestrationRejection::GraphMissing)
    );
}

fn graph_state() -> OrchestrationState {
    reduce_orchestration(
        OrchestrationState::default(),
        OrchestrationEvent::Fanout(FanoutRequest {
            graph: graph(),
            expected_parent_run_id: AgentChatRunId("run-1".into()),
        }),
    )
    .0
}

fn assert_review_rejection(
    state: OrchestrationState,
    change: impl FnOnce(&mut CrossReviewRequest),
    expected: OrchestrationRejection,
) {
    let mut request = review();
    request.idempotency_key = format!("review-{expected:?}");
    change(&mut request);
    assert_eq!(
        reduce_orchestration(state, OrchestrationEvent::CrossReview(request)).1,
        OrchestrationEffect::Rejected(expected)
    );
}

#[test]
fn fanout_and_cross_review_reject_each_exact_invalid_fence() {
    let mut invalid = graph();
    invalid.idempotency_key.clear();
    assert_eq!(
        reduce_orchestration(
            OrchestrationState::default(),
            OrchestrationEvent::Fanout(FanoutRequest {
                graph: invalid,
                expected_parent_run_id: AgentChatRunId("run-1".into()),
            })
        )
        .1,
        OrchestrationEffect::Rejected(OrchestrationRejection::InvalidRequest)
    );
    let mut altered = graph();
    altered.idempotency_key = "fanout-2".into();
    assert_eq!(
        reduce_orchestration(
            graph_state(),
            OrchestrationEvent::Fanout(FanoutRequest {
                graph: altered,
                expected_parent_run_id: AgentChatRunId("run-1".into()),
            })
        )
        .1,
        OrchestrationEffect::Rejected(OrchestrationRejection::GraphAlreadyExists)
    );
    assert_review_rejection(
        graph_state(),
        |value| value.graph_id = "other".into(),
        OrchestrationRejection::GraphMismatch,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.expected_parent_run_id = AgentChatRunId("other".into()),
        OrchestrationRejection::ParentRunMismatch,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.host_epoch = HostEpoch(2),
        OrchestrationRejection::StaleHostEpoch,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.goal_revision = 2,
        OrchestrationRejection::StaleGoalRevision,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.policy_revision = 2,
        OrchestrationRejection::StalePolicyRevision,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.expected_graph_revision = 2,
        OrchestrationRejection::StaleGraphRevision,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.candidate.node_id = "missing".into(),
        OrchestrationRejection::CandidateMissing,
    );
}

#[test]
fn cross_review_rejects_candidate_and_reviewer_mismatches() {
    let mut pending = graph();
    pending.nodes[0].status = TaskNodeStatus::Pending;
    assert_review_rejection(
        OrchestrationState::new(Some(pending)),
        |_| {},
        OrchestrationRejection::CandidateNotTerminal,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.candidate.node_revision = 3,
        OrchestrationRejection::CandidateRevisionMismatch,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.candidate.artifact_digest_sha256 = "c".repeat(64),
        OrchestrationRejection::CandidateArtifactMismatch,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.candidate.base_revision_digest_sha256 = "c".repeat(64),
        OrchestrationRejection::CandidateBaseRevisionMismatch,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.reviewer.role = TaskRole::Implementer,
        OrchestrationRejection::ReviewerNotReviewer,
    );
    assert_review_rejection(
        graph_state(),
        |value| value.reviewer.node_id = "candidate".into(),
        OrchestrationRejection::DuplicateNode,
    );
}

#[test]
fn successful_review_is_idempotent_but_conflicting_reuse_is_rejected() {
    let (state, persisted) =
        reduce_orchestration(graph_state(), OrchestrationEvent::CrossReview(review()));
    assert!(matches!(persisted, OrchestrationEffect::Persist(_)));
    assert!(matches!(
        reduce_orchestration(state.clone(), OrchestrationEvent::CrossReview(review())).1,
        OrchestrationEffect::Unchanged(_)
    ));
    let mut conflict = review();
    conflict.expected_graph_revision = 2;
    assert_eq!(
        reduce_orchestration(state, OrchestrationEvent::CrossReview(conflict)).1,
        OrchestrationEffect::Rejected(OrchestrationRejection::IdempotencyConflict)
    );
}
