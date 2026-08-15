//! Adapter joining workspace identity queries to the public persistence port.

use gent_ports::{LedgerError, WorkspaceLedger};
use gent_types::{RepositoryRecord, WorkspaceRecord, WorktreeRecord};

use super::{SqliteLedger, workspaces};

impl WorkspaceLedger for SqliteLedger {
    fn create_workspace(&self, workspace: &WorkspaceRecord) -> Result<(), LedgerError> {
        workspaces::create_workspace(self, workspace)
    }

    fn create_repository(&self, repository: &RepositoryRecord) -> Result<(), LedgerError> {
        workspaces::create_repository(self, repository)
    }

    fn create_worktree(&self, worktree: &WorktreeRecord) -> Result<(), LedgerError> {
        workspaces::create_worktree(self, worktree)
    }

    fn find_workspace(&self, workspace_id: &str) -> Result<Option<WorkspaceRecord>, LedgerError> {
        workspaces::find_workspace(self, workspace_id)
    }

    fn find_worktree(&self, worktree_id: &str) -> Result<Option<WorktreeRecord>, LedgerError> {
        workspaces::find_worktree(self, worktree_id)
    }

    fn list_workspace_repositories(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<RepositoryRecord>, LedgerError> {
        workspaces::list_repositories(self, workspace_id)
    }

    fn list_repository_worktrees(
        &self,
        repository_id: &str,
    ) -> Result<Vec<WorktreeRecord>, LedgerError> {
        workspaces::list_worktrees(self, repository_id)
    }
}
