use gent_ports::{
    AgentChatLedger, GoalLedger, OrchestrationLedger, OrchestrationWrite, PolicyLedger,
    WorkspaceLedger,
};
use gent_types::{
    AgentChatConversationCreate, AgentChatConversationId, AgentChatEffort, AgentChatMode,
    AgentChatProvider, AgentChatRunId, AgentChatSelection, CrossReviewRequest, FanoutRequest,
    GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord, GoalStatus, HarnessProfileRef, HostEpoch,
    PermissionMode, PolicyRecord, PolicyScope, ReceiptId, RepositoryRecord, ReviewCandidate,
    TaskGraph, TaskGraphBinding, TaskGraphFactKind, TaskNode, TaskNodeSpec, TaskNodeStatus,
    TaskRole, WorkspaceRecord, WorktreePolicy,
};

use super::SqliteLedger;

fn ledger() -> SqliteLedger {
    let ledger = SqliteLedger::in_memory().unwrap();
    ledger
        .create_agent_chat_conversation(&AgentChatConversationCreate {
            receipt_id: ReceiptId("conversation-receipt".into()),
            idempotency_key: "conversation-key".into(),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
            selection: selection(AgentChatProvider::Codex),
        })
        .unwrap();
    ledger
        .create_workspace(&WorkspaceRecord {
            workspace_id: "workspace-1".into(),
            canonical_path: "/workspace".into(),
        })
        .unwrap();
    ledger
        .create_repository(&RepositoryRecord {
            repository_id: "repo-1".into(),
            workspace_id: "workspace-1".into(),
            canonical_path: "/workspace/repo".into(),
        })
        .unwrap();
    ledger
        .save_policy(&PolicyRecord {
            policy_id: "policy-1".into(),
            workspace_id: "workspace-1".into(),
            scope: PolicyScope::ProviderPermissions,
            revision: 1,
            mode: PermissionMode::Default,
            allowed_tools: vec![],
            allowed_categories: vec![],
        })
        .unwrap();
    ledger
        .create_goal(&GoalRecord {
            schema_version: GOAL_SCHEMA_VERSION,
            binding: GoalBinding {
                goal_id: "goal-1".into(),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
            },
            revision: 1,
            status: GoalStatus::Active,
            summary: "Finish safely".into(),
        })
        .unwrap();
    ledger
}
fn selection(provider: AgentChatProvider) -> AgentChatSelection {
    AgentChatSelection {
        provider,
        model: "test".into(),
        effort: AgentChatEffort::Medium,
        mode: AgentChatMode::Agent,
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
        selection: selection(provider),
        input_artifact_digests: vec![],
        depends_on: vec![],
        worktree: WorktreePolicy::Isolated,
        retry_budget: 1,
    }
}
fn graph() -> TaskGraph {
    let mut candidate = spec("candidate", TaskRole::Implementer, AgentChatProvider::Codex);
    candidate.input_artifact_digests = vec!["b".repeat(64)];
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
            spec: candidate,
            revision: 2,
            status: TaskNodeStatus::Completed,
            result_artifact_digest: Some("a".repeat(64)),
        }],
    }
}
fn review(key: &str) -> CrossReviewRequest {
    CrossReviewRequest {
        graph_id: "graph-1".into(),
        expected_graph_revision: 1,
        expected_parent_run_id: AgentChatRunId("run-1".into()),
        host_epoch: HostEpoch(1),
        goal_revision: 1,
        policy_revision: 1,
        idempotency_key: key.into(),
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
fn fanout_rechecks_durable_fences_and_is_retry_safe() {
    let ledger = ledger();
    let request = FanoutRequest {
        graph: graph(),
        expected_parent_run_id: AgentChatRunId("run-1".into()),
    };
    assert!(matches!(
        ledger.apply_fanout(&request).unwrap(),
        OrchestrationWrite::Created(_)
    ));
    assert!(matches!(
        ledger.apply_fanout(&request).unwrap(),
        OrchestrationWrite::Current(_)
    ));
    assert_eq!(ledger.task_graph("graph-1").unwrap(), Some(graph()));
    let mut changed = request;
    changed.graph.idempotency_key = "fanout-2".into();
    assert!(ledger.apply_fanout(&changed).is_err());
}

#[test]
fn cross_review_is_atomic_cross_provider_and_revision_fenced() {
    let ledger = ledger();
    ledger
        .apply_fanout(&FanoutRequest {
            graph: graph(),
            expected_parent_run_id: AgentChatRunId("run-1".into()),
        })
        .unwrap();
    let request = review("review-1");
    assert!(
        matches!(ledger.apply_cross_review(&request).unwrap(), OrchestrationWrite::Updated(next) if next.nodes.len() == 2)
    );
    assert!(
        matches!(ledger.apply_cross_review(&request).unwrap(), OrchestrationWrite::Current(next) if next.revision == 2)
    );
    let mut stale = review("review-2");
    stale.expected_graph_revision = 1;
    assert!(ledger.apply_cross_review(&stale).is_err());
    let mut same = review("review-3");
    same.reviewer.profile.provider = AgentChatProvider::Codex;
    same.reviewer.selection.provider = AgentChatProvider::Codex;
    assert!(ledger.apply_cross_review(&same).is_err());
}

#[test]
fn graph_is_reconstructed_from_bounded_ordered_immutable_facts() {
    let ledger = ledger();
    ledger
        .apply_fanout(&FanoutRequest {
            graph: graph(),
            expected_parent_run_id: AgentChatRunId("run-1".into()),
        })
        .unwrap();
    ledger.apply_cross_review(&review("review-1")).unwrap();
    let first = ledger.task_graph_facts("graph-1", 0, 2).unwrap();
    assert_eq!(first.facts.len(), 2);
    assert!(matches!(
        first.facts[0].kind,
        TaskGraphFactKind::Created { .. }
    ));
    let cursor = first.next_after_cursor.unwrap();
    let second = ledger.task_graph_facts("graph-1", cursor, 2).unwrap();
    assert_eq!(second.facts.len(), 2);
    assert!(matches!(
        second.facts[0].kind,
        TaskGraphFactKind::ReviewAccepted { .. }
    ));
    assert!(second.next_after_cursor.is_none());
    assert_eq!(ledger.task_graph("graph-1").unwrap().unwrap().revision, 2);
}
