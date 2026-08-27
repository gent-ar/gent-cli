use crate::local_ipc::connect_and_negotiate;
use clap::Subcommand;
use gent_protocol::{WORKSPACE_GIT_CAPABILITY, WorkspaceGitFrame};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Subcommand)]
pub(crate) enum WorkspaceGitCommand {
    Status {
        #[arg(long, default_value = "workspace-1")]
        workspace_id: String,
    },
    SubRepos {
        #[arg(long, default_value = "workspace-1")]
        workspace_id: String,
    },
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: WorkspaceGitCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    let frame = match command {
        WorkspaceGitCommand::Status { workspace_id } => WorkspaceGitFrame::StatusRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            workspace_id,
        },
        WorkspaceGitCommand::SubRepos { workspace_id } => WorkspaceGitFrame::SubReposRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            workspace_id,
        },
    };
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities.0.iter().any(|value| value == WORKSPACE_GIT_CAPABILITY) {
        return Err("workspace git capability is unavailable".into());
    }
    gent_protocol::write_json_frame(&mut stream, &frame).await?;
    let reply: WorkspaceGitFrame = gent_protocol::read_json_frame(&mut stream).await?;
    Ok(serde_json::to_value(reply)?)
}
