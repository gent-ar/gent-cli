//! Durable Git operation identities and lifecycle phases.

use serde::{Deserialize, Serialize};

/// The declared intent of a Git operation. Execution policy lives outside this type.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitOperationKind {
    Status,
    Commit,
    CreateWorktree,
    RemoveWorktree,
}

/// Monotonic durable lifecycle for one Git operation request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitOperationPhase {
    Requested,
    Running,
    Succeeded,
    Failed,
    Interrupted,
}

impl GitOperationPhase {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Interrupted)
    }
}

/// Immutable identity and current durable phase for one worktree-scoped operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOperationRecord {
    pub operation_id: String,
    pub worktree_id: String,
    pub run_id: String,
    pub kind: GitOperationKind,
    pub phase: GitOperationPhase,
}
