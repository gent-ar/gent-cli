//! Durable identity boundary for workspace, repository, and worktree selection.

use gent_types::{RepositoryRecord, WorkspaceRecord, WorktreeRecord};

use crate::LedgerError;

/// Persistence boundary for immutable local filesystem identities.
pub trait WorkspaceLedger: Send + Sync {
    /// Creates a workspace identity exactly once.
    ///
    /// # Errors
    /// Returns an error when the identity is invalid, occupied, or cannot be persisted.
    fn create_workspace(&self, workspace: &WorkspaceRecord) -> Result<(), LedgerError>;

    /// Creates a repository under an existing workspace identity.
    ///
    /// # Errors
    /// Returns an error when its identity or parent is invalid, or persistence fails.
    fn create_repository(&self, repository: &RepositoryRecord) -> Result<(), LedgerError>;

    /// Creates a worktree under an existing repository identity.
    ///
    /// # Errors
    /// Returns an error when its identity or parent is invalid, or persistence fails.
    fn create_worktree(&self, worktree: &WorktreeRecord) -> Result<(), LedgerError>;

    /// Reads one workspace identity.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_workspace(&self, workspace_id: &str) -> Result<Option<WorkspaceRecord>, LedgerError>;

    /// Reads one durable worktree identity, including its daemon-selected canonical path.
    ///
    /// # Errors
    /// Returns an error when the implementation cannot resolve worktree identities.
    fn find_worktree(&self, worktree_id: &str) -> Result<Option<WorktreeRecord>, LedgerError> {
        let _ = worktree_id;
        Err(LedgerError::Invariant(
            "worktree lookup is unavailable".into(),
        ))
    }

    /// Lists repositories in durable creation order.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn list_workspace_repositories(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<RepositoryRecord>, LedgerError>;

    /// Lists worktrees in durable creation order.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn list_repository_worktrees(
        &self,
        repository_id: &str,
    ) -> Result<Vec<WorktreeRecord>, LedgerError>;
}
