use crate::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
    AgentChatSelection, FanoutRequest, GoalStatus, HarnessProfileRef, HostEpoch,
    OrchestrationContractError, TaskGraph, TaskGraphBinding, TaskNode, TaskNodeSpec,
    TaskNodeStatus, TaskRole, WorktreePolicy,
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
        idempotency_key: "receipt-1".into(),
        nodes: vec![TaskNode {
            spec: TaskNodeSpec {
                node_id: "candidate".into(),
                role: TaskRole::Implementer,
                profile: HarnessProfileRef {
                    profile_id: "codex-1".into(),
                    revision: 1,
                    provider: AgentChatProvider::Codex,
                },
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "test".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
                input_artifact_digests: vec![],
                depends_on: vec![],
                worktree: WorktreePolicy::Isolated,
                retry_budget: 1,
            },
            revision: 1,
            status: TaskNodeStatus::Completed,
            result_artifact_digest: Some("a".repeat(64)),
        }],
    }
}

#[test]
fn graph_rejects_cycles_and_unknown_client_fields() {
    let mut cyclic = graph();
    cyclic.nodes[0].spec.depends_on = vec!["candidate".into()];
    assert_eq!(
        cyclic.validate(),
        Err(OrchestrationContractError::InvalidGraph)
    );
    let value = serde_json::json!({ "graph": graph(), "expectedParentRunId": "run-1", "providerSessionId": "no" });
    assert!(serde_json::from_value::<FanoutRequest>(value).is_err());
    let _ = GoalStatus::Active;
}
