//! Daemon-only workspace resolution for read-only git status and sub-repository discovery.

use gent_git::executor::SystemGitExecutor;
use gent_ports::{GitExecutor, Ledger, WorkspaceLedger};
use gent_protocol::WorkspaceGitFrame;
use gent_runtime::Coordinator;
use gent_types::WorkspaceGitReport;

pub(crate) fn exchange<L>(
    coordinator: &Coordinator<L>,
    frame: WorkspaceGitFrame,
) -> Result<WorkspaceGitFrame, String>
where
    L: Ledger + WorkspaceLedger,
{
    match frame {
        WorkspaceGitFrame::StatusRequest {
            request_id,
            workspace_id,
        } => {
            let workspace = resolve(coordinator, &workspace_id)?;
            let report = status(&workspace);
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
                crate::workspace_git_sub_repos::discover(std::path::Path::new(&workspace))
                    .unwrap_or_default();
            Ok(WorkspaceGitFrame::SubRepos {
                request_id,
                workspace_id,
                canonical_paths,
            })
        }
        WorkspaceGitFrame::Status { .. } | WorkspaceGitFrame::SubRepos { .. } => {
            Err("workspace git response frames are server-only".into())
        }
    }
}

fn resolve<L: Ledger + WorkspaceLedger>(
    coordinator: &Coordinator<L>,
    workspace_id: &str,
) -> Result<String, String> {
    coordinator
        .workspace(workspace_id)
        .map_err(|error| error.to_string())?
        .map(|workspace| workspace.canonical_path)
        .ok_or_else(|| "workspace was not found".to_owned())
}

/// Reads the workspace's git report. `None` distinguishes "not a git repository" from an error;
/// a transient execution failure is treated the same way rather than failing the whole read.
pub(crate) fn status(workspace_canonical_path: &str) -> Option<WorkspaceGitReport> {
    let root = SystemGitExecutor
        .repository_root(workspace_canonical_path)
        .ok()?;
    let report = SystemGitExecutor.report(&root).ok()?;
    Some(WorkspaceGitReport {
        repository_root: root,
        branch: report.branch,
        files: report.files,
        worktrees: report.worktrees,
    })
}
