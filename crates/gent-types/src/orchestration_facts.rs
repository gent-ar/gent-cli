//! Immutable facts and bounded pages for Gent-owned orchestration graphs.

use serde::{Deserialize, Serialize};

use crate::{HostEpoch, ReviewCandidate, TaskGraphBinding, TaskNode};

/// One append-only graph fact. Cursors are ordered only within `graph_id`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGraphFact {
    pub graph_id: String,
    pub cursor: u64,
    pub revision: u64,
    pub idempotency_key: String,
    pub kind: TaskGraphFactKind,
}

/// The facts sufficient to reconstruct a graph without replacing a whole document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum TaskGraphFactKind {
    Created {
        binding: TaskGraphBinding,
        host_epoch: HostEpoch,
        idempotency_key: String,
    },
    NodeAdded {
        node: TaskNode,
    },
    NodeStatusChanged {
        node_id: String,
        node_revision: u64,
        status: crate::TaskNodeStatus,
        result_artifact_digest: Option<String>,
    },
    ReviewAccepted {
        candidate: ReviewCandidate,
        reviewer_node_id: String,
    },
}

/// A bounded, cursor-ordered page of immutable graph facts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGraphFactPage {
    pub facts: Vec<TaskGraphFact>,
    pub next_after_cursor: Option<u64>,
}
