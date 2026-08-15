//! `SQLite` persistence for immutable workspace hierarchy identities.

use gent_ports::LedgerError;
use gent_types::{RepositoryRecord, WorkspaceRecord, WorktreeRecord};
use rusqlite::{OptionalExtension, params};

use super::SqliteLedger;
use super::queries::storage_error;

pub(super) fn create_workspace(
    ledger: &SqliteLedger,
    workspace: &WorkspaceRecord,
) -> Result<(), LedgerError> {
    require_identity(
        &workspace.workspace_id,
        &workspace.canonical_path,
        "workspace",
    )?;
    insert(
        ledger,
        "INSERT INTO workspaces (workspace_id, canonical_path) VALUES (?1, ?2)",
        params![workspace.workspace_id, workspace.canonical_path],
    )
}

pub(super) fn create_repository(
    ledger: &SqliteLedger,
    repository: &RepositoryRecord,
) -> Result<(), LedgerError> {
    require_identity(
        &repository.repository_id,
        &repository.canonical_path,
        "repository",
    )?;
    insert(
        ledger,
        "INSERT INTO repositories (repository_id, workspace_id, canonical_path) VALUES (?1, ?2, ?3)",
        params![
            repository.repository_id,
            repository.workspace_id,
            repository.canonical_path
        ],
    )
}

pub(super) fn create_worktree(
    ledger: &SqliteLedger,
    worktree: &WorktreeRecord,
) -> Result<(), LedgerError> {
    require_identity(&worktree.worktree_id, &worktree.canonical_path, "worktree")?;
    insert(
        ledger,
        "INSERT INTO worktrees (worktree_id, repository_id, canonical_path) VALUES (?1, ?2, ?3)",
        params![
            worktree.worktree_id,
            worktree.repository_id,
            worktree.canonical_path
        ],
    )
}

pub(super) fn find_workspace(
    ledger: &SqliteLedger,
    workspace_id: &str,
) -> Result<Option<WorkspaceRecord>, LedgerError> {
    let connection = ledger.lock()?;
    connection
        .query_row(
            "SELECT workspace_id, canonical_path FROM workspaces WHERE workspace_id = ?1",
            [workspace_id],
            |row| {
                Ok(WorkspaceRecord {
                    workspace_id: row.get(0)?,
                    canonical_path: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(storage_error)
}

pub(super) fn find_worktree(
    ledger: &SqliteLedger,
    worktree_id: &str,
) -> Result<Option<WorktreeRecord>, LedgerError> {
    let connection = ledger.lock()?;
    connection
        .query_row(
            "SELECT worktree_id, repository_id, canonical_path FROM worktrees WHERE worktree_id = ?1",
            [worktree_id],
            |row| {
                Ok(WorktreeRecord {
                    worktree_id: row.get(0)?,
                    repository_id: row.get(1)?,
                    canonical_path: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(storage_error)
}

pub(super) fn list_repositories(
    ledger: &SqliteLedger,
    workspace_id: &str,
) -> Result<Vec<RepositoryRecord>, LedgerError> {
    let connection = ledger.lock()?;
    let mut statement = connection
        .prepare("SELECT repository_id, workspace_id, canonical_path FROM repositories WHERE workspace_id = ?1 ORDER BY rowid")
        .map_err(storage_error)?;
    let rows = statement
        .query_map([workspace_id], |row| {
            Ok(RepositoryRecord {
                repository_id: row.get(0)?,
                workspace_id: row.get(1)?,
                canonical_path: row.get(2)?,
            })
        })
        .map_err(storage_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
}

pub(super) fn list_worktrees(
    ledger: &SqliteLedger,
    repository_id: &str,
) -> Result<Vec<WorktreeRecord>, LedgerError> {
    let connection = ledger.lock()?;
    let mut statement = connection
        .prepare("SELECT worktree_id, repository_id, canonical_path FROM worktrees WHERE repository_id = ?1 ORDER BY rowid")
        .map_err(storage_error)?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok(WorktreeRecord {
                worktree_id: row.get(0)?,
                repository_id: row.get(1)?,
                canonical_path: row.get(2)?,
            })
        })
        .map_err(storage_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
}

fn insert<P: rusqlite::Params>(
    ledger: &SqliteLedger,
    sql: &str,
    values: P,
) -> Result<(), LedgerError> {
    ledger
        .lock()?
        .execute(sql, values)
        .map(|_| ())
        .map_err(storage_error)
}

fn require_identity(id: &str, path: &str, kind: &str) -> Result<(), LedgerError> {
    if id.is_empty() || path.is_empty() {
        return Err(LedgerError::Invariant(format!(
            "{kind} identity and canonical path are required"
        )));
    }
    Ok(())
}
