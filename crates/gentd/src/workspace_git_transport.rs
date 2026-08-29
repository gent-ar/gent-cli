use crate::{api::RuntimeApi, transport::write_error};
use gent_protocol::{WORKSPACE_GIT_CAPABILITY, WorkspaceGitFrame, write_json_frame};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

pub(crate) async fn dispatch<S, R>(
    stream: &mut S,
    runtime: &R,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    if !capabilities
        .0
        .iter()
        .any(|item| item == WORKSPACE_GIT_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<WorkspaceGitFrame>(raw.clone()) else {
        return Ok(false);
    };
    if !matches!(
        frame,
        WorkspaceGitFrame::StatusRequest { .. }
            | WorkspaceGitFrame::SubReposRequest { .. }
            | WorkspaceGitFrame::ResolveRequest { .. }
    ) {
        write_error(
            stream,
            "invalidWorkspaceGit",
            "workspace git response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    match runtime.workspace_git(frame) {
        Ok(reply) => write_json_frame(stream, &reply).await?,
        Err(message) => write_error(stream, "workspaceGitRejected", &message).await?,
    }
    Ok(true)
}
