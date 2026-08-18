use super::{
    MAX_ORCHESTRATION_FRAME_BYTES, ORCHESTRATION_CAPABILITY, OrchestrationFrame,
    OrchestrationFrameError,
};
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRunId,
    AgentChatSelection, FanoutRequest, HarnessProfileRef, HostEpoch, TaskGraph, TaskGraphBinding,
    TaskNode, TaskNodeSpec, TaskNodeStatus, TaskRole, WorktreePolicy,
};
use serde_json::json;

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
            repository_id: "repository-1".into(),
            base_revision_digest_sha256: "b".repeat(64),
        },
        revision: 1,
        host_epoch: HostEpoch(1),
        idempotency_key: "receipt-1".into(),
        nodes: vec![TaskNode {
            spec: TaskNodeSpec {
                node_id: "task-1".into(),
                role: TaskRole::Implementer,
                profile: HarnessProfileRef {
                    profile_id: "codex-default".into(),
                    revision: 1,
                    provider: AgentChatProvider::Codex,
                },
                selection: AgentChatSelection {
                    provider: AgentChatProvider::Codex,
                    model: "gpt-5".into(),
                    effort: AgentChatEffort::Medium,
                    mode: AgentChatMode::Agent,
                },
                input_artifact_digests: Vec::new(),
                depends_on: Vec::new(),
                worktree: WorktreePolicy::Isolated,
                retry_budget: 0,
            },
            revision: 1,
            status: TaskNodeStatus::Pending,
            result_artifact_digest: None,
        }],
    }
}

#[test]
fn fanout_is_typed_and_capability_gated() {
    let graph = graph();
    let frame = OrchestrationFrame::Fanout {
        request_id: "request-1".into(),
        request: FanoutRequest {
            expected_parent_run_id: graph.binding.root_run_id.clone(),
            graph,
        },
    };
    assert_eq!(frame.validate(), Ok(()));
    assert_eq!(ORCHESTRATION_CAPABILITY, "orchestration-v1");
}

#[test]
fn frames_reject_unknown_provider_fields() {
    let frame = json!({
        "type": "graphRead", "body": {
            "requestId": "request-1", "conversationId": "conversation-1", "graphId": "graph-1",
            "providerCommand": "never"
        }
    });
    assert!(serde_json::from_value::<OrchestrationFrame>(frame).is_err());
}

#[test]
fn graph_reply_cannot_cross_its_read_scope() {
    let graph = graph();
    let frame = OrchestrationFrame::Graph {
        request_id: "request-1".into(),
        conversation_id: AgentChatConversationId("conversation-2".into()),
        graph_id: graph.binding.graph_id.clone(),
        graph: Some(graph),
    };
    assert_eq!(
        frame.validate(),
        Err(OrchestrationFrameError::GraphScopeMismatch)
    );
}

#[test]
fn byte_budget_applies_to_graph_replies() {
    let mut graph = graph();
    graph.nodes[0].spec.selection.model = "x".repeat(MAX_ORCHESTRATION_FRAME_BYTES);
    let frame = OrchestrationFrame::GraphSaved {
        request_id: "request-1".into(),
        graph,
    };
    assert!(serde_json::to_vec(&frame).unwrap().len() > MAX_ORCHESTRATION_FRAME_BYTES);
    assert_eq!(frame.validate(), Err(OrchestrationFrameError::TooLarge));
}
