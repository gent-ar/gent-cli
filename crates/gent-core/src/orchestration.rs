//! Pure graph and cross-review reduction for the future Gent-owned orchestrator.

use gent_types::{
    CrossReviewRequest, FanoutRequest, TaskGraph, TaskNode, TaskNodeSpec, TaskNodeStatus,
};

/// Durable state reconstructed by a future ledger before one orchestration command.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OrchestrationState {
    graph: Option<TaskGraph>,
    applied: Vec<OrchestrationEvent>,
}

impl OrchestrationState {
    #[must_use]
    pub const fn new(graph: Option<TaskGraph>) -> Self {
        Self {
            graph,
            applied: Vec::new(),
        }
    }
    #[must_use]
    pub fn graph(&self) -> Option<&TaskGraph> {
        self.graph.as_ref()
    }
}

/// Closed client-facing graph command accepted by the pure reducer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrchestrationEvent {
    Fanout(FanoutRequest),
    CrossReview(CrossReviewRequest),
}

/// One candidate durable write or an explicit no-write outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrchestrationEffect {
    Persist(TaskGraph),
    Unchanged(TaskGraph),
    Rejected(OrchestrationRejection),
}

/// Provider-free rejection vocabulary for graph, fence, and idempotency failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrchestrationRejection {
    InvalidRequest,
    GraphAlreadyExists,
    GraphMissing,
    GraphMismatch,
    ParentRunMismatch,
    StaleGraphRevision,
    StaleHostEpoch,
    StaleGoalRevision,
    StalePolicyRevision,
    IdempotencyConflict,
    CandidateMissing,
    CandidateNotTerminal,
    CandidateRevisionMismatch,
    CandidateArtifactMismatch,
    CandidateBaseRevisionMismatch,
    ReviewerMustCrossProvider,
    ReviewerNotReviewer,
    DuplicateNode,
}

/// Reduces a typed graph command without I/O, process access, clocks, or provider output.
#[must_use]
pub fn reduce_orchestration(
    state: OrchestrationState,
    event: OrchestrationEvent,
) -> (OrchestrationState, OrchestrationEffect) {
    if !valid_event(&event) {
        return reject(state, OrchestrationRejection::InvalidRequest);
    }
    if let Some(previous) = state
        .applied
        .iter()
        .find(|previous| idempotency(previous) == idempotency(&event))
    {
        return if previous == &event {
            unchanged(state)
        } else {
            reject(state, OrchestrationRejection::IdempotencyConflict)
        };
    }
    let applied = event.clone();
    let (mut next, effect) = match event {
        OrchestrationEvent::Fanout(request) => fanout(state, request),
        OrchestrationEvent::CrossReview(request) => cross_review(state, request),
    };
    if matches!(effect, OrchestrationEffect::Persist(_)) {
        next.applied.push(applied);
    }
    (next, effect)
}

fn fanout(
    state: OrchestrationState,
    request: FanoutRequest,
) -> (OrchestrationState, OrchestrationEffect) {
    if request.validate().is_err() {
        return reject(state, OrchestrationRejection::InvalidRequest);
    }
    match state.graph.as_ref() {
        None => persist(state, request.graph),
        Some(current)
            if current.idempotency_key == request.graph.idempotency_key
                && current == &request.graph =>
        {
            unchanged(state)
        }
        Some(current) if current.idempotency_key == request.graph.idempotency_key => {
            reject(state, OrchestrationRejection::IdempotencyConflict)
        }
        Some(_) => reject(state, OrchestrationRejection::GraphAlreadyExists),
    }
}

fn cross_review(
    state: OrchestrationState,
    request: CrossReviewRequest,
) -> (OrchestrationState, OrchestrationEffect) {
    if request.validate().is_err() {
        return reject(state, OrchestrationRejection::InvalidRequest);
    }
    let Some(graph) = state.graph.as_ref() else {
        return reject(state, OrchestrationRejection::GraphMissing);
    };
    if graph.binding.graph_id != request.graph_id {
        return reject(state, OrchestrationRejection::GraphMismatch);
    }
    if graph.binding.root_run_id != request.expected_parent_run_id {
        return reject(state, OrchestrationRejection::ParentRunMismatch);
    }
    if graph.host_epoch != request.host_epoch {
        return reject(state, OrchestrationRejection::StaleHostEpoch);
    }
    if graph.binding.goal_revision != request.goal_revision {
        return reject(state, OrchestrationRejection::StaleGoalRevision);
    }
    if graph.binding.policy_revision != request.policy_revision {
        return reject(state, OrchestrationRejection::StalePolicyRevision);
    }
    if graph.revision != request.expected_graph_revision {
        return reject(state, OrchestrationRejection::StaleGraphRevision);
    }
    let Some(candidate) = graph
        .nodes
        .iter()
        .find(|node| node.spec.node_id == request.candidate.node_id)
    else {
        return reject(state, OrchestrationRejection::CandidateMissing);
    };
    if candidate.status != TaskNodeStatus::Completed {
        return reject(state, OrchestrationRejection::CandidateNotTerminal);
    }
    if candidate.revision != request.candidate.node_revision {
        return reject(state, OrchestrationRejection::CandidateRevisionMismatch);
    }
    if candidate.result_artifact_digest.as_deref()
        != Some(&request.candidate.artifact_digest_sha256)
    {
        return reject(state, OrchestrationRejection::CandidateArtifactMismatch);
    }
    if graph.binding.base_revision_digest_sha256 != request.candidate.base_revision_digest_sha256 {
        return reject(state, OrchestrationRejection::CandidateBaseRevisionMismatch);
    }
    if candidate.spec.profile.provider == request.reviewer.profile.provider {
        return reject(state, OrchestrationRejection::ReviewerMustCrossProvider);
    }
    if !matches!(request.reviewer.role, gent_types::TaskRole::Reviewer) {
        return reject(state, OrchestrationRejection::ReviewerNotReviewer);
    }
    if graph
        .nodes
        .iter()
        .any(|node| node.spec.node_id == request.reviewer.node_id)
    {
        return reject(state, OrchestrationRejection::DuplicateNode);
    }
    let mut next = graph.clone();
    next.revision = next.revision.saturating_add(1);
    next.nodes.push(TaskNode {
        spec: review_spec(request.reviewer, &request.candidate.node_id),
        revision: 1,
        status: TaskNodeStatus::Pending,
        result_artifact_digest: None,
    });
    persist(state, next)
}

fn review_spec(mut reviewer: TaskNodeSpec, candidate_id: &str) -> TaskNodeSpec {
    reviewer.depends_on = vec![candidate_id.into()];
    reviewer.input_artifact_digests = reviewer.input_artifact_digests.into_iter().collect();
    reviewer
}
fn idempotency(event: &OrchestrationEvent) -> &str {
    match event {
        OrchestrationEvent::Fanout(request) => &request.graph.idempotency_key,
        OrchestrationEvent::CrossReview(request) => &request.idempotency_key,
    }
}
fn valid_event(event: &OrchestrationEvent) -> bool {
    match event {
        OrchestrationEvent::Fanout(request) => request.validate().is_ok(),
        OrchestrationEvent::CrossReview(request) => request.validate().is_ok(),
    }
}
fn persist(
    mut state: OrchestrationState,
    graph: TaskGraph,
) -> (OrchestrationState, OrchestrationEffect) {
    state.graph = Some(graph.clone());
    (state, OrchestrationEffect::Persist(graph))
}
fn unchanged(state: OrchestrationState) -> (OrchestrationState, OrchestrationEffect) {
    let graph = state.graph.clone().expect("checked graph");
    (state, OrchestrationEffect::Unchanged(graph))
}
fn reject(
    state: OrchestrationState,
    reason: OrchestrationRejection,
) -> (OrchestrationState, OrchestrationEffect) {
    (state, OrchestrationEffect::Rejected(reason))
}
