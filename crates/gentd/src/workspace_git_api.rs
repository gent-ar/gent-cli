//! Daemon-only workspace resolution for read-only git status and sub-repository discovery.

#[path = "single_flight.rs"]
mod single_flight;

use gent_git::executor::SystemGitExecutor;
use gent_ports::{GitExecutor, GitExecutorError, Ledger, WorkspaceLedger};
use gent_protocol::WorkspaceGitFrame;
use gent_runtime::Coordinator;
use gent_types::WorkspaceGitReport;
use single_flight::SingleFlight;
use std::{path::Path, sync::LazyLock};

use crate::{agent_chat_intent_error::AgentChatIntentError, workspace_identity::require_directory};

pub(crate) fn rejected(error: impl std::fmt::Display) -> AgentChatIntentError {
    AgentChatIntentError::coded("workspaceGitRejected", &error.to_string())
}

pub(crate) fn exchange<L>(
    coordinator: &Coordinator<L>,
    frame: WorkspaceGitFrame,
) -> Result<WorkspaceGitFrame, AgentChatIntentError>
where
    L: Ledger + WorkspaceLedger,
{
    match frame {
        WorkspaceGitFrame::StatusRequest {
            request_id,
            workspace_id,
        } => {
            let workspace = resolve(coordinator, &workspace_id)?;
            let report = status(&workspace).map_err(rejected)?;
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
            let canonical_paths =
                gent_git::workspace_repositories(Path::new(&workspace)).map_err(rejected)?;
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
            require_directory(&workspace_path)?;
            let workspace = crate::workspace_identity::CanonicalWorkspace::from_path(Path::new(
                &workspace_path,
            ))
            .map_err(|error| rejected(format!("workspace path is unavailable: {error:?}")))?;
            let record = workspace.record().clone();
            if coordinator
                .workspace(&record.workspace_id)
                .map_err(rejected)?
                .is_none()
            {
                coordinator.create_workspace(&record).map_err(rejected)?;
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
            Err(rejected("workspace git response frames are server-only"))
        }
    }
}

fn resolve<L: Ledger + WorkspaceLedger>(
    coordinator: &Coordinator<L>,
    workspace_id: &str,
) -> Result<String, AgentChatIntentError> {
    let path = coordinator
        .workspace(workspace_id)
        .map_err(rejected)?
        .map(|workspace| workspace.canonical_path)
        .ok_or_else(|| {
            AgentChatIntentError::coded(
                "workspaceNotFound",
                &format!("workspace {workspace_id} was not found"),
            )
        })?;
    require_directory(&path)?;
    Ok(path)
}

fn status(workspace_canonical_path: &str) -> Result<Option<WorkspaceGitReport>, String> {
    static STATUS: LazyLock<SingleFlight<Result<Option<WorkspaceGitReport>, String>>> =
        LazyLock::new(SingleFlight::new);
    STATUS.run(workspace_canonical_path, || {
        uncoalesced_status(workspace_canonical_path)
    })
}

fn uncoalesced_status(
    workspace_canonical_path: &str,
) -> Result<Option<WorkspaceGitReport>, String> {
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
