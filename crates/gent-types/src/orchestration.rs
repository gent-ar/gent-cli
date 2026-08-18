//! Strict provider-neutral values for planned Gent-native task orchestration.

use serde::{Deserialize, Serialize};

use crate::{
    AgentChatConversationId, AgentChatProvider, AgentChatRunId, AgentChatSelection, HostEpoch,
};

const MAX_ID: usize = 128;
const MAX_ITEMS: usize = 64;
const MAX_DIGEST: usize = 64;

/// Immutable catalog reference for an approved worker profile, never a command or endpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessProfileRef {
    pub profile_id: String,
    pub revision: u64,
    pub provider: AgentChatProvider,
}

/// Declarative task role; roles grant no authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum TaskRole {
    Planner,
    Implementer,
    Researcher,
    Reviewer,
    Custom { role_id: String },
}

/// The only task worktree policy accepted by this foundation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum WorktreePolicy {
    Isolated,
}

/// One unstarted task supplied to a typed fanout command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskNodeSpec {
    pub node_id: String,
    pub role: TaskRole,
    pub profile: HarnessProfileRef,
    pub selection: AgentChatSelection,
    pub input_artifact_digests: Vec<String>,
    pub depends_on: Vec<String>,
    pub worktree: WorktreePolicy,
    pub retry_budget: u8,
}

/// Closed durable node state. Dispatch and attempts are deliberately out of scope here.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum TaskNodeStatus {
    Pending,
    Completed,
    Failed,
    Cancelled,
}

/// Graph node with a daemon-owned immutable completion artifact reference, when terminal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskNode {
    pub spec: TaskNodeSpec,
    pub revision: u64,
    pub status: TaskNodeStatus,
    pub result_artifact_digest: Option<String>,
}

/// Immutable graph identity and all fences that must be rechecked durably later.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGraphBinding {
    pub graph_id: String,
    pub conversation_id: AgentChatConversationId,
    pub root_run_id: AgentChatRunId,
    pub goal_id: String,
    pub goal_revision: u64,
    pub policy_id: String,
    pub policy_revision: u64,
    pub workspace_id: String,
    pub repository_id: String,
    pub base_revision_digest_sha256: String,
}

/// A durable graph snapshot suitable for pure transition reduction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGraph {
    pub binding: TaskGraphBinding,
    pub revision: u64,
    pub host_epoch: HostEpoch,
    pub idempotency_key: String,
    pub nodes: Vec<TaskNode>,
}

/// Typed `/fanout` input. It never carries provider sessions, plans, commands, or raw output.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FanoutRequest {
    pub graph: TaskGraph,
    pub expected_parent_run_id: AgentChatRunId,
}

/// Exact immutable candidate reference accepted by a future cross-review graph transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewCandidate {
    pub node_id: String,
    pub node_revision: u64,
    pub artifact_digest_sha256: String,
    pub base_revision_digest_sha256: String,
}

/// Typed `/cross-review` input. Reviewer task data is declarative and cross-provider only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CrossReviewRequest {
    pub graph_id: String,
    pub expected_graph_revision: u64,
    pub expected_parent_run_id: AgentChatRunId,
    pub host_epoch: HostEpoch,
    pub goal_revision: u64,
    pub policy_revision: u64,
    pub idempotency_key: String,
    pub candidate: ReviewCandidate,
    pub reviewer: TaskNodeSpec,
}

/// Contract failure that exposes no provider-local material.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum OrchestrationContractError {
    #[error("orchestration metadata is invalid or exceeds its bound")]
    InvalidMetadata,
    #[error("task graph dependencies are invalid")]
    InvalidGraph,
}

impl TaskGraph {
    /// Validates graph shape before any durable or provider action.
    ///
    /// # Errors
    /// Returns a bounded metadata or graph-shape failure.
    pub fn validate(&self) -> Result<(), OrchestrationContractError> {
        valid_binding(&self.binding)?;
        if self.revision == 0
            || self.host_epoch.0 == 0
            || !valid_id(&self.idempotency_key)
            || self.nodes.is_empty()
            || self.nodes.len() > MAX_ITEMS
        {
            return Err(OrchestrationContractError::InvalidMetadata);
        }
        let ids = self
            .nodes
            .iter()
            .map(|node| node.spec.node_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if ids.len() != self.nodes.len()
            || self.nodes.iter().any(|node| !valid_node(node, &ids))
            || has_cycle(&self.nodes)
        {
            return Err(OrchestrationContractError::InvalidGraph);
        }
        Ok(())
    }
}

impl FanoutRequest {
    /// Validates the initial graph and its exact root-run binding.
    ///
    /// # Errors
    /// Returns a bounded metadata or graph-shape failure.
    pub fn validate(&self) -> Result<(), OrchestrationContractError> {
        self.graph.validate()?;
        (self.graph.binding.root_run_id == self.expected_parent_run_id)
            .then_some(())
            .ok_or(OrchestrationContractError::InvalidMetadata)
    }
}
impl CrossReviewRequest {
    /// Validates only bounded, provider-neutral review request metadata.
    ///
    /// # Errors
    /// Returns a bounded metadata failure.
    pub fn validate(&self) -> Result<(), OrchestrationContractError> {
        if self.expected_graph_revision == 0
            || self.host_epoch.0 == 0
            || self.goal_revision == 0
            || self.policy_revision == 0
            || !valid_id(&self.graph_id)
            || !valid_id(&self.idempotency_key)
            || !valid_id(&self.candidate.node_id)
            || !digest(&self.candidate.artifact_digest_sha256)
            || !digest(&self.candidate.base_revision_digest_sha256)
        {
            return Err(OrchestrationContractError::InvalidMetadata);
        }
        valid_spec(&self.reviewer, &std::collections::BTreeSet::new())
    }
}

fn valid_binding(value: &TaskGraphBinding) -> Result<(), OrchestrationContractError> {
    if value.goal_revision == 0
        || value.policy_revision == 0
        || !digest(&value.base_revision_digest_sha256)
        || [
            value.graph_id.as_str(),
            &value.conversation_id.0,
            &value.root_run_id.0,
            value.goal_id.as_str(),
            value.policy_id.as_str(),
            value.workspace_id.as_str(),
            value.repository_id.as_str(),
        ]
        .iter()
        .any(|value| !valid_id(value))
    {
        Err(OrchestrationContractError::InvalidMetadata)
    } else {
        Ok(())
    }
}
fn valid_node(node: &TaskNode, ids: &std::collections::BTreeSet<&str>) -> bool {
    node.revision > 0
        && valid_spec(&node.spec, ids).is_ok()
        && match node.status {
            TaskNodeStatus::Completed => node.result_artifact_digest.as_deref().is_some_and(digest),
            _ => node.result_artifact_digest.is_none(),
        }
}
fn valid_spec(
    value: &TaskNodeSpec,
    ids: &std::collections::BTreeSet<&str>,
) -> Result<(), OrchestrationContractError> {
    let node_id = value.node_id.as_str();
    if !valid_id(&value.node_id)
        || value.profile.revision == 0
        || !valid_id(&value.profile.profile_id)
        || value.profile.provider != value.selection.provider
        || value.input_artifact_digests.len() > MAX_ITEMS
        || value.depends_on.len() > MAX_ITEMS
        || value
            .input_artifact_digests
            .iter()
            .any(|value| !digest(value))
        || value.depends_on.iter().any(|dependency| {
            !valid_id(dependency)
                || dependency == node_id
                || (!ids.is_empty() && !ids.contains(dependency.as_str()))
        })
        || matches!(&value.role, TaskRole::Custom { role_id } if !valid_id(role_id))
    {
        Err(OrchestrationContractError::InvalidMetadata)
    } else {
        Ok(())
    }
}
fn has_cycle(nodes: &[TaskNode]) -> bool {
    fn visit<'a>(
        id: &'a str,
        nodes: &'a [TaskNode],
        visiting: &mut std::collections::BTreeSet<&'a str>,
        visited: &mut std::collections::BTreeSet<&'a str>,
    ) -> bool {
        if !visited.insert(id) {
            return visiting.contains(id);
        }
        visiting.insert(id);
        let cyclic = nodes
            .iter()
            .find(|node| node.spec.node_id == id)
            .is_some_and(|node| {
                node.spec
                    .depends_on
                    .iter()
                    .any(|next| visit(next, nodes, visiting, visited))
            });
        visiting.remove(id);
        cyclic
    }
    let mut visiting = std::collections::BTreeSet::new();
    let mut visited = std::collections::BTreeSet::new();
    nodes
        .iter()
        .any(|node| visit(&node.spec.node_id, nodes, &mut visiting, &mut visited))
}
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_ID && !value.chars().any(char::is_control)
}
fn digest(value: &str) -> bool {
    value.len() == MAX_DIGEST && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
