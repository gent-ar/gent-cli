//! Capability-gated attachment response writer for local IPC.

use gent_protocol::{AttachmentFrame, write_json_frame};
use tokio::io::AsyncWrite;

use crate::api::RuntimeApi;

/// Dispatches a parsed attachment frame through the daemon API and writes one typed response.
pub(crate) async fn write<S, R>(
    stream: &mut S,
    runtime: &R,
    frame: AttachmentFrame,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    match runtime.attachment(frame) {
        Ok(response) => write_json_frame(stream, &response).await?,
        Err(message) => {
            write_json_frame(
                stream,
                &AttachmentFrame::Error {
                    code: "invalidAttachment".into(),
                    message,
                },
            )
            .await?;
        }
    }
    Ok(true)
}
