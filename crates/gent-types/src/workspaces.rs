//! Durable workspace, repository, and worktree identities.

use serde::{Deserialize, Serialize};

/// A user-selected root that may contain one or more repositories.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub workspace_id: String,
    pub canonical_path: String,
}

/// An immutable repository identity belonging to one workspace.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryRecord {
    pub repository_id: String,
    pub workspace_id: String,
    pub canonical_path: String,
}

/// An immutable worktree identity belonging to one repository.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRecord {
    pub worktree_id: String,
    pub repository_id: String,
    pub canonical_path: String,
}
