//! Coordinator calls for durable workspace selection, without filesystem mutation.

use gent_ports::WorkspaceLedger;
use gent_types::{RepositoryRecord, WorkspaceRecord, WorktreeRecord};

use crate::{Coordinator, RuntimeError};

impl<L> Coordinator<L>
where
    L: gent_ports::Ledger + WorkspaceLedger,
{
    /// Persists a user-selected workspace identity.
    ///
    /// # Errors
    /// Returns an error when the identity is invalid or persistence fails.
    pub fn create_workspace(&self, workspace: &WorkspaceRecord) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_workspace(workspace)?)
    }

    /// Persists a repository identity under its selected workspace.
    ///
    /// # Errors
    /// Returns an error when the parent identity is missing or persistence fails.
    pub fn create_repository(&self, repository: &RepositoryRecord) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_repository(repository)?)
    }

    /// Persists a worktree identity; Git creation remains outside this boundary.
    ///
    /// # Errors
    /// Returns an error when the parent identity is missing or persistence fails.
    pub fn create_worktree(&self, worktree: &WorktreeRecord) -> Result<(), RuntimeError> {
        Ok(self.ledger.create_worktree(worktree)?)
    }

    /// Lists the durable repository identities for one workspace.
    ///
    /// # Errors
    /// Returns an error when the hierarchy cannot be read.
    pub fn workspace_repositories(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<RepositoryRecord>, RuntimeError> {
        Ok(self.ledger.list_workspace_repositories(workspace_id)?)
    }

    /// Lists the durable worktree identities for one repository.
    ///
    /// # Errors
    /// Returns an error when the hierarchy cannot be read.
    pub fn repository_worktrees(
        &self,
        repository_id: &str,
    ) -> Result<Vec<WorktreeRecord>, RuntimeError> {
        Ok(self.ledger.list_repository_worktrees(repository_id)?)
    }
}
