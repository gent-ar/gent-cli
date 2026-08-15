//! Attachment-frame adapter over the runtime's opaque staging service.

use base64::Engine;
use gent_protocol::AttachmentFrame;
use gent_runtime::AttachmentService;

/// Executes one negotiated attachment request without exposing blob paths to transport code.
pub(crate) fn handle(
    attachments: &AttachmentService<gent_store::SqliteLedger, gent_store::FileAttachmentBlobs>,
    frame: AttachmentFrame,
) -> Result<AttachmentFrame, String> {
    match frame {
        AttachmentFrame::Begin { transfer } => attachments
            .begin(&transfer)
            .map_err(|error| error.to_string())
            .map(|transfer| AttachmentFrame::Transfer { transfer }),
        AttachmentFrame::Chunk {
            operation,
            offset,
            data_base64,
        } => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data_base64)
                .map_err(|error| error.to_string())?;
            attachments
                .append(&operation, offset, &bytes)
                .map_err(|error| error.to_string())
                .map(|transfer| AttachmentFrame::Transfer { transfer })
        }
        AttachmentFrame::Commit { operation } => attachments
            .commit(&operation)
            .map_err(|error| error.to_string())
            .map(|transfer| AttachmentFrame::Transfer { transfer }),
        AttachmentFrame::Resume { attachment_id } => attachments
            .resume(&attachment_id)
            .map_err(|error| error.to_string())
            .map(|transfer| AttachmentFrame::Transfer { transfer }),
        AttachmentFrame::Transfer { .. } | AttachmentFrame::Error { .. } => {
            Err("attachment response frames are not valid requests".into())
        }
    }
}
