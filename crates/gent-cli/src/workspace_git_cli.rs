use crate::{
    cli_error::{CliError, Failure},
    local_ipc::{LocalStream, connect_and_negotiate},
};
use clap::{Args, Subcommand};
use gent_protocol::{WORKSPACE_GIT_CAPABILITY, WorkspaceGitFrame};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub(crate) enum WorkspaceGitCommand {
    #[command(about = "Show the Git status, branches, and worktrees of a workspace")]
    Status {
        #[command(flatten)]
        workspace: WorkspaceTarget,
    },
    #[command(about = "List the repositories a workspace contains when it holds more than one")]
    SubRepos {
        #[command(flatten)]
        workspace: WorkspaceTarget,
    },
}

#[derive(Debug, Args)]
pub(crate) struct WorkspaceTarget {
    #[arg(
        long,
        conflicts_with = "path",
        help = "Durable workspace id reported by conversation summaries"
    )]
    workspace_id: Option<String>,
    #[arg(
        long,
        help = "Workspace directory to inspect [default: the current directory]"
    )]
    path: Option<PathBuf>,
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: WorkspaceGitCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|value| value == WORKSPACE_GIT_CAPABILITY)
    {
        return Err(CliError::new(
            Failure::Unavailable,
            "gentd does not expose workspace Git status",
        )
        .into());
    }
    let (workspace, status) = match command {
        WorkspaceGitCommand::Status { workspace } => (workspace, true),
        WorkspaceGitCommand::SubRepos { workspace } => (workspace, false),
    };
    let workspace_id = match workspace.workspace_id {
        Some(workspace_id) => workspace_id,
        None => resolve(&mut stream, workspace.path).await?,
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let frame = if status {
        WorkspaceGitFrame::StatusRequest {
            request_id,
            workspace_id,
        }
    } else {
        WorkspaceGitFrame::SubReposRequest {
            request_id,
            workspace_id,
        }
    };
    Ok(serde_json::to_value(exchange(&mut stream, &frame).await?)?)
}

async fn resolve(
    stream: &mut LocalStream,
    path: Option<PathBuf>,
) -> Result<String, Box<dyn std::error::Error>> {
    let path = match path {
        Some(path) => path,
        None => std::env::current_dir()?,
    };
    let frame = WorkspaceGitFrame::ResolveRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        workspace_path: path.display().to_string(),
    };
    match exchange(stream, &frame).await? {
        WorkspaceGitFrame::Resolved { workspace_id, .. } => Ok(workspace_id),
        _ => Err("gentd did not resolve the workspace".into()),
    }
}

async fn exchange(
    stream: &mut LocalStream,
    frame: &WorkspaceGitFrame,
) -> Result<WorkspaceGitFrame, Box<dyn std::error::Error>> {
    gent_protocol::write_json_frame(stream, frame).await?;
    let raw: Value = gent_protocol::read_json_frame(stream).await?;
    if let Some(error) = CliError::from_reply(&raw) {
        return Err(error.into());
    }
    let reply: WorkspaceGitFrame = serde_json::from_value(raw)
        .map_err(|_| "gentd returned an unrecognized workspace Git reply")?;
    let correlated = match (frame, &reply) {
        (
            WorkspaceGitFrame::StatusRequest { request_id, .. },
            WorkspaceGitFrame::Status {
                request_id: reply, ..
            },
        )
        | (
            WorkspaceGitFrame::SubReposRequest { request_id, .. },
            WorkspaceGitFrame::SubRepos {
                request_id: reply, ..
            },
        )
        | (
            WorkspaceGitFrame::ResolveRequest { request_id, .. },
            WorkspaceGitFrame::Resolved {
                request_id: reply, ..
            },
        ) => request_id == reply,
        _ => false,
    };
    if !correlated {
        return Err("gentd returned an uncorrelated workspace Git reply".into());
    }
    Ok(reply)
}

#[cfg(all(test, unix))]
#[path = "workspace_git_cli_tests.rs"]
mod tests;
