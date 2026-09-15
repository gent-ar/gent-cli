//! Daemon-only workspace resolution for read-only git status and sub-repository discovery.

use gent_git::executor::SystemGitExecutor;
use gent_ports::{GitExecutor, GitExecutorError, Ledger, WorkspaceLedger};
use gent_protocol::WorkspaceGitFrame;
use gent_runtime::Coordinator;
use gent_types::WorkspaceGitReport;
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceGitRejection {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl From<String> for WorkspaceGitRejection {
    fn from(message: String) -> Self {
        Self {
            code: "workspaceGitRejected",
            message,
        }
    }
}

impl From<&str> for WorkspaceGitRejection {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

pub(crate) fn exchange<L>(
    coordinator: &Coordinator<L>,
    frame: WorkspaceGitFrame,
) -> Result<WorkspaceGitFrame, WorkspaceGitRejection>
where
    L: Ledger + WorkspaceLedger,
{
    match frame {
        WorkspaceGitFrame::StatusRequest {
            request_id,
            workspace_id,
        } => {
            let workspace = resolve(coordinator, &workspace_id)?;
            let report = status(&workspace)?;
            Ok(WorkspaceGitFrame::Status {
                request_id,
                workspace_id,
                report,
            })
        }
        WorkspaceGitFrame::SubReposRequest {
            request_id,
            workspace_id,
        } => {
            let workspace = resolve(coordinator, &workspace_id)?;
            let canonical_paths = gent_git::workspace_repositories(Path::new(&workspace))
                .map_err(|error| error.to_string())?;
            Ok(WorkspaceGitFrame::SubRepos {
                request_id,
                workspace_id,
                canonical_paths,
            })
        }
        WorkspaceGitFrame::ResolveRequest {
            request_id,
            workspace_path,
        } => {
            let workspace = crate::workspace_identity::CanonicalWorkspace::from_path(Path::new(
                &workspace_path,
            ))
            .map_err(|error| format!("workspace path is unavailable: {error:?}"))?;
            let record = workspace.record().clone();
            if coordinator
                .workspace(&record.workspace_id)
                .map_err(|error| error.to_string())?
                .is_none()
            {
                coordinator
                    .create_workspace(&record)
                    .map_err(|error| error.to_string())?;
            }
            Ok(WorkspaceGitFrame::Resolved {
                request_id,
                workspace_id: record.workspace_id,
                canonical_path: record.canonical_path,
            })
        }
        WorkspaceGitFrame::Status { .. }
        | WorkspaceGitFrame::SubRepos { .. }
        | WorkspaceGitFrame::Resolved { .. } => {
            Err("workspace git response frames are server-only".into())
        }
    }
}

fn resolve<L: Ledger + WorkspaceLedger>(
    coordinator: &Coordinator<L>,
    workspace_id: &str,
) -> Result<String, WorkspaceGitRejection> {
    coordinator
        .workspace(workspace_id)
        .map_err(|error| error.to_string())?
        .map(|workspace| workspace.canonical_path)
        .ok_or_else(|| WorkspaceGitRejection {
            code: "workspaceNotFound",
            message: format!("workspace {workspace_id} was not found"),
        })
}

pub(crate) fn status(workspace_canonical_path: &str) -> Result<Option<WorkspaceGitReport>, String> {
    let root = match SystemGitExecutor.repository_root(workspace_canonical_path) {
        Ok(root) => root,
        Err(GitExecutorError::NotRepository) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let report = SystemGitExecutor
        .report(&root)
        .map_err(|error| error.to_string())?;
    Ok(Some(WorkspaceGitReport {
        repository_root: root,
        branch: report.branch,
        files: report.files,
        worktrees: report.worktrees,
        recent_commits: report.recent_commits,
        branches: report.branches,
        stashes: report.stashes,
        remote_status: report.remote_status,
    }))
}
