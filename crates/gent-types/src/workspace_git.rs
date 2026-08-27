//! Read-only durable projection of a workspace's git status and worktrees.

use serde::{Deserialize, Serialize};

/// One file's porcelain status, matching `git status --porcelain=v1 -z` semantics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitFileStatus {
    pub index_status: char,
    pub worktree_status: char,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_path: Option<String>,
}

/// One entry from `git worktree list`, real git-backed state rather than durable metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitWorktree {
    pub canonical_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    pub is_detached: bool,
    pub is_locked: bool,
}

/// A workspace's full git report, or absent when the workspace is not inside a git repository.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGitReport {
    pub repository_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub files: Vec<WorkspaceGitFileStatus>,
    pub worktrees: Vec<WorkspaceGitWorktree>,
}
