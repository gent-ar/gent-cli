use std::path::{Path, PathBuf};

use base64::Engine;
use gent_protocol::{ATTACHMENTS_CAPABILITY, AttachmentFrame, read_json_frame, write_json_frame};
use gent_types::{
    AttachmentMetadata, AttachmentOperation, AttachmentState, AttachmentTransfer, HostEpoch,
    ReceiptId,
};
use sha2::{Digest, Sha256};

use crate::local_ipc::{LocalStream, connect_and_negotiate};

const MAX_ATTACHMENT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CHUNK_BYTES: usize = 256 * 1024;

pub(crate) async fn stage(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    paths: &[PathBuf],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|value| value == ATTACHMENTS_CAPABILITY)
    {
        return Err("daemon does not support attachment staging".into());
    }
    let mut ids = Vec::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        ids.push(stage_one(&mut stream, path, index).await?);
    }
    Ok(ids)
}

async fn stage_one(
    stream: &mut LocalStream,
    path: &Path,
    index: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() || u64::try_from(bytes.len())? > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "attachment {} must be between 1 byte and 64 MiB",
            path.display()
        )
        .into());
    }
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let attachment_id = format!("attachment-{digest}-{index}");
    let display_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or("attachment filename must be valid UTF-8")?;
    let transfer = round_trip(
        stream,
        AttachmentFrame::Begin {
            transfer: AttachmentTransfer {
                metadata: AttachmentMetadata {
                    attachment_id: attachment_id.clone(),
                    display_name: display_name.into(),
                    media_type: media_type(path),
                    byte_len: bytes.len() as u64,
                    digest_sha256: digest.clone(),
                    storage_key: format!("sha256/{digest}"),
                },
                staging_key: format!("staging/{attachment_id}"),
                receipt_id: ReceiptId(format!("receipt-{attachment_id}")),
                idempotency_key: attachment_id.clone(),
                host_epoch: HostEpoch(0),
                state: AttachmentState::Uploading,
                received_bytes: 0,
            },
        },
    )
    .await?;
    let mut transfer = transfer;
    for (offset, chunk) in bytes.chunks(MAX_CHUNK_BYTES).enumerate() {
        let offset = offset * MAX_CHUNK_BYTES;
        let operation_id = format!("{attachment_id}-chunk-{offset}");
        transfer = round_trip(
            stream,
            AttachmentFrame::Chunk {
                operation: operation(&transfer, &operation_id),
                offset: offset as u64,
                data_base64: base64::engine::general_purpose::STANDARD.encode(chunk),
            },
        )
        .await?;
    }
    let commit_id = format!("{attachment_id}-commit");
    transfer = round_trip(
        stream,
        AttachmentFrame::Commit {
            operation: operation(&transfer, &commit_id),
        },
    )
    .await?;
    if transfer.state != AttachmentState::Available {
        return Err(format!("attachment {} did not become available", path.display()).into());
    }
    Ok(attachment_id)
}

async fn round_trip(
    stream: &mut LocalStream,
    frame: AttachmentFrame,
) -> Result<AttachmentTransfer, Box<dyn std::error::Error>> {
    write_json_frame(stream, &frame).await?;
    match read_json_frame::<_, AttachmentFrame>(stream).await? {
        AttachmentFrame::Transfer { transfer } => Ok(transfer),
        AttachmentFrame::Error { message, .. } => Err(message.into()),
        _ => Err("daemon returned an invalid attachment response".into()),
    }
}

fn operation(transfer: &AttachmentTransfer, operation_id: &str) -> AttachmentOperation {
    AttachmentOperation {
        attachment_id: transfer.metadata.attachment_id.clone(),
        transfer_receipt_id: transfer.receipt_id.clone(),
        transfer_idempotency_key: transfer.idempotency_key.clone(),
        receipt_id: ReceiptId(format!("receipt-{operation_id}")),
        idempotency_key: operation_id.into(),
        host_epoch: HostEpoch(0),
    }
}

fn media_type(path: &Path) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("txt") => "text/plain",
        Some("json") => "application/json",
        Some("pdf") => "application/pdf",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    }
    .into()
}
