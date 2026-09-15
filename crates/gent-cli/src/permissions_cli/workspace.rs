use std::path::PathBuf;

use gent_protocol::{
    WORKSPACE_GIT_CAPABILITY, WorkspaceGitFrame, read_json_frame, write_json_frame,
};
use serde_json::Value;

use crate::cli_error::CliError;
use crate::local_ipc::connect_and_negotiate;

pub(super) async fn resolve(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    path: Option<PathBuf>,
) -> Result<String, Box<dyn std::error::Error>> {
    let path = match path {
        Some(path) => path,
        None => std::env::current_dir()?,
    };
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == WORKSPACE_GIT_CAPABILITY)
    {
        return Err("gentd cannot resolve workspaces; upgrade gentd".into());
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    write_json_frame(
        &mut stream,
        &WorkspaceGitFrame::ResolveRequest {
            request_id: request_id.clone(),
            workspace_path: path.display().to_string(),
        },
    )
    .await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Some(error) = CliError::from_reply(&raw) {
        return Err(error.into());
    }
    match serde_json::from_value(raw)? {
        WorkspaceGitFrame::Resolved {
            request_id: reply,
            workspace_id,
            ..
        } if reply == request_id => Ok(workspace_id),
        _ => Err("gentd did not resolve the workspace".into()),
    }
}
