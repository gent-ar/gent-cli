use gent_ports::{LedgerError, OrchestrationLedger, OrchestrationWrite};
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
    AgentChatSelection, CrossReviewRequest, FanoutRequest, HarnessProfileRef, HostEpoch,
    ReviewCandidate, TaskGraph, TaskGraphBinding, TaskNode, TaskNodeSpec, TaskNodeStatus, TaskRole,
    WorktreePolicy,
};

use crate::{OrchestrationAuthority, OrchestrationResult, OrchestrationService};

#[derive(Debug)]
struct PanicLedger;
impl OrchestrationLedger for PanicLedger {
    fn task_graph(&self, _: &str) -> Result<Option<TaskGraph>, LedgerError> {
        panic!("observer read")
    }
    fn apply_fanout(&self, _: &FanoutRequest) -> Result<OrchestrationWrite, LedgerError> {
        panic!("observer write")
    }
    fn apply_cross_review(
        &self,
        _: &CrossReviewRequest,
    ) -> Result<OrchestrationWrite, LedgerError> {
        panic!("observer write")
    }
}
#[derive(Debug)]
struct Ledger(TaskGraph);
impl OrchestrationLedger for Ledger {
    fn task_graph(&self, _: &str) -> Result<Option<TaskGraph>, LedgerError> {
        Ok(Some(self.0.clone()))
    }
    fn apply_fanout(&self, _: &FanoutRequest) -> Result<OrchestrationWrite, LedgerError> {
        Ok(OrchestrationWrite::Created(self.0.clone()))
    }
    fn apply_cross_review(
        &self,
        _: &CrossReviewRequest,
    ) -> Result<OrchestrationWrite, LedgerError> {
        Ok(OrchestrationWrite::Updated(self.0.clone()))
    }
}
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
            revision: 1,
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
            node_revision: 1,
            artifact_digest_sha256: "a".repeat(64),
            base_revision_digest_sha256: "b".repeat(64),
        },
        reviewer: spec("reviewer", TaskRole::Reviewer, AgentChatProvider::Claude),
    }
}

#[test]
fn observer_never_reads_or_writes_orchestration_storage() {
    let service = OrchestrationService::new(PanicLedger, OrchestrationAuthority::Observer);
    assert_eq!(
        service.graph("graph-1").unwrap(),
        OrchestrationResult::DeniedObserver
    );
    assert_eq!(
        service
            .fanout(&FanoutRequest {
                graph: graph(),
                expected_parent_run_id: AgentChatRunId("run-1".into())
            })
            .unwrap(),
        OrchestrationResult::DeniedObserver
    );
    assert_eq!(
        service.cross_review(&review()).unwrap(),
        OrchestrationResult::DeniedObserver
    );
}
#[test]
fn approved_service_maps_graph_reads_and_atomic_writes() {
    let graph = graph();
    let service =
        OrchestrationService::new(Ledger(graph.clone()), OrchestrationAuthority::Approved);
    assert_eq!(
        service.graph("graph-1").unwrap(),
        OrchestrationResult::Graph(Box::new(graph.clone()))
    );
    assert_eq!(
        service
            .fanout(&FanoutRequest {
                graph: graph.clone(),
                expected_parent_run_id: AgentChatRunId("run-1".into())
            })
            .unwrap(),
        OrchestrationResult::Graph(Box::new(graph.clone()))
    );
    assert_eq!(
        service.cross_review(&review()).unwrap(),
        OrchestrationResult::Graph(Box::new(graph))
    );
}
