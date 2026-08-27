use crate::{api::RuntimeApi, transport::write_error};
use gent_protocol::{WORKSPACE_DOCUMENTS_CAPABILITY, WorkspaceDocumentsFrame, write_json_frame};
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
        .any(|item| item == WORKSPACE_DOCUMENTS_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<WorkspaceDocumentsFrame>(raw.clone()) else {
        return Ok(false);
    };
    if frame.validate().is_err() || !matches!(frame, WorkspaceDocumentsFrame::List { .. }) {
        write_error(
            stream,
            "invalidWorkspaceDocuments",
            "workspace document request is invalid",
        )
        .await?;
        return Ok(true);
    }
    match runtime.workspace_documents(frame) {
        Ok(reply) => write_json_frame(stream, &reply).await?,
        Err(message) => write_error(stream, "workspaceDocumentsRejected", &message).await?,
    }
    Ok(true)
}
