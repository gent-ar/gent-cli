//! Opaque attachment values. They intentionally never contain source paths or bytes.

use serde::{Deserialize, Serialize};

use crate::{HostEpoch, ReceiptId};

/// Availability of a content-addressed attachment reference.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentState {
    Uploading,
    Available,
    Rejected,
}

/// Immutable metadata for content staged by the daemon.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMetadata {
    pub attachment_id: String,
    pub display_name: String,
    pub media_type: String,
    pub byte_len: u64,
    /// Lowercase hexadecimal SHA-256 digest, without a transport prefix.
    pub digest_sha256: String,
    /// Opaque content-addressed key, never an absolute filesystem path.
    pub storage_key: String,
}

/// Durable receipt-scoped progress for one local attachment transfer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentTransfer {
    pub metadata: AttachmentMetadata,
    /// Opaque, transfer-owned staging identity; it is never a local path.
    pub staging_key: String,
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
    pub state: AttachmentState,
    pub received_bytes: u64,
}

/// Receipt and fence identity required by every follow-up attachment mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentOperation {
    pub attachment_id: String,
    pub receipt_id: ReceiptId,
    pub idempotency_key: String,
    pub host_epoch: HostEpoch,
}

/// Immutable association of an available attachment with a durable turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnAttachment {
    pub turn_id: String,
    pub attachment_id: String,
    /// Fences the association against a superseded daemon host.
    pub host_epoch: HostEpoch,
}
