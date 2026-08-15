//! Attachment transfer orchestration over durable metadata and opaque blob ports.

use gent_core::{
    IngressMode as CoreIngressMode, IngressState, accept_chunk, commit, validate_attachment,
    validate_ingress, validate_staging_key,
};
use gent_ports::{AttachmentBlobStore, AttachmentClaim, AttachmentLedger, IngressMode, Ledger};
use gent_types::{AttachmentOperation, AttachmentState, AttachmentTransfer};
use sha2::{Digest, Sha256};

use crate::RuntimeError;
use crate::attachment_receipts::{claim as claim_receipt, settle as settle_receipt};

/// Coordinates retry-safe local attachment staging without importing filesystem or IPC types.
#[derive(Clone, Debug)]
pub struct AttachmentService<L, B> {
    ledger: L,
    blobs: B,
}

impl<L, B> AttachmentService<L, B>
where
    L: Ledger + AttachmentLedger,
    B: AttachmentBlobStore,
{
    #[must_use]
    pub fn new(ledger: L, blobs: B) -> Self {
        Self { ledger, blobs }
    }

    /// Validates ingress and claims a durable local staging transfer.
    ///
    /// # Errors
    /// Returns an error before writing bytes when metadata, epoch, or ingress is invalid.
    pub fn begin(&self, transfer: &AttachmentTransfer) -> Result<AttachmentTransfer, RuntimeError> {
        validate_attachment(&transfer.metadata)
            .map_err(|error| gent_ports::LedgerError::Invariant(error.to_string()))?;
        validate_staging_key(&transfer.staging_key)
            .map_err(|error| gent_ports::LedgerError::Invariant(error.to_string()))?;
        if transfer.state != AttachmentState::Uploading || transfer.received_bytes != 0 {
            return Err(gent_ports::LedgerError::Invariant(
                "attachment begin must start uploading at offset zero".into(),
            )
            .into());
        }
        self.ensure_ingress(transfer)?;
        let payload = serde_json::json!({
            "attachmentId": transfer.metadata.attachment_id,
            "digestSha256": transfer.metadata.digest_sha256,
            "byteLen": transfer.metadata.byte_len,
        });
        if !claim_receipt(
            &self.ledger,
            &transfer.receipt_id,
            &transfer.idempotency_key,
            transfer.host_epoch,
            "attachmentBegin",
            payload,
        )? {
            return self.resume(&transfer.metadata.attachment_id);
        }
        let result = match self.ledger.claim_attachment(transfer)? {
            AttachmentClaim::Created(value) | AttachmentClaim::Existing(value) => Ok(value),
        };
        settle_receipt(
            &self.ledger,
            &transfer.receipt_id,
            &transfer.idempotency_key,
            transfer.host_epoch,
            "attachmentBegin",
            result,
        )
    }

    /// Appends one validated sequential chunk; retries are safe when a prior blob write succeeded first.
    ///
    /// # Errors
    /// Returns an error for stale ingress, missing metadata, or invalid ordered progress.
    pub fn append(
        &self,
        operation: &AttachmentOperation,
        offset: u64,
        bytes: &[u8],
    ) -> Result<AttachmentTransfer, RuntimeError> {
        let current = self
            .ledger
            .find_attachment(&operation.attachment_id)?
            .ok_or_else(|| {
                gent_ports::LedgerError::Invariant("attachment does not exist".into())
            })?;
        Self::require_operation(&current, operation)?;
        let payload = serde_json::json!({
            "attachmentId": operation.attachment_id,
            "offset": offset,
            "bytesSha256": format!("{:x}", Sha256::digest(bytes)),
        });
        if current.state == AttachmentState::Available {
            return self.settle_available_operation(operation, "attachmentAppend", payload);
        }
        self.ensure_ingress(&current)?;
        let byte_len = u64::try_from(bytes.len())
            .map_err(|_| gent_ports::LedgerError::Invariant("chunk length overflow".into()))?;
        let next = if offset.checked_add(byte_len) == Some(current.received_bytes) {
            Ok(None)
        } else {
            accept_chunk(&current, offset, byte_len)
                .map(Some)
                .map_err(|error| gent_ports::LedgerError::Invariant(error.to_string()).into())
        };
        if !claim_receipt(
            &self.ledger,
            &operation.receipt_id,
            &operation.idempotency_key,
            operation.host_epoch,
            "attachmentAppend",
            payload,
        )? {
            return Ok(current);
        }
        let result = match next {
            Ok(None) => self
                .blobs
                .append_attachment_chunk(&current.staging_key, offset, bytes)
                .map(|()| current.clone())
                .map_err(RuntimeError::from),
            Ok(Some(next)) => self
                .blobs
                .append_attachment_chunk(&current.staging_key, offset, bytes)
                .map_err(RuntimeError::from)
                .and_then(|()| {
                    self.ledger
                        .replace_attachment(&current, &next)
                        .map_err(RuntimeError::from)
                })
                .map(|()| next),
            Err(error) => Err(error),
        };
        settle_receipt(
            &self.ledger,
            &operation.receipt_id,
            &operation.idempotency_key,
            operation.host_epoch,
            "attachmentAppend",
            result,
        )
    }

    /// Verifies the complete staged digest, promotes content, then durably marks it available.
    ///
    /// # Errors
    /// Returns an error without availability when the byte count or digest differs.
    pub fn commit(
        &self,
        operation: &AttachmentOperation,
    ) -> Result<AttachmentTransfer, RuntimeError> {
        let current = self
            .ledger
            .find_attachment(&operation.attachment_id)?
            .ok_or_else(|| {
                gent_ports::LedgerError::Invariant("attachment does not exist".into())
            })?;
        Self::require_operation(&current, operation)?;
        let payload = serde_json::json!({ "attachmentId": operation.attachment_id });
        if current.state == AttachmentState::Available {
            return self.settle_available_operation(operation, "attachmentCommit", payload);
        }
        self.ensure_ingress(&current)?;
        let prepared = self
            .blobs
            .attachment_digest(&current.staging_key, &current.metadata.storage_key)
            .map_err(RuntimeError::from)
            .and_then(|(size, digest)| {
                if size != current.received_bytes {
                    return Err(gent_ports::LedgerError::Invariant(
                        "staged attachment size differs from durable progress".into(),
                    )
                    .into());
                }
                commit(&current, &digest)
                    .map_err(|error| gent_ports::LedgerError::Invariant(error.to_string()).into())
            });
        if !claim_receipt(
            &self.ledger,
            &operation.receipt_id,
            &operation.idempotency_key,
            operation.host_epoch,
            "attachmentCommit",
            payload,
        )? {
            return Ok(current);
        }
        let result = prepared.and_then(|next| {
            self.blobs
                .commit_attachment_blob(&current.staging_key, &current.metadata.storage_key)
                .map_err(RuntimeError::from)
                .and_then(|()| {
                    self.ledger
                        .replace_attachment(&current, &next)
                        .map_err(RuntimeError::from)
                })
                .map(|()| next)
        });
        settle_receipt(
            &self.ledger,
            &operation.receipt_id,
            &operation.idempotency_key,
            operation.host_epoch,
            "attachmentCommit",
            result,
        )
    }

    /// Returns durable progress without opening ingress or touching staged bytes.
    ///
    /// # Errors
    /// Returns an error when the attachment is unknown or metadata cannot be read.
    pub fn resume(&self, attachment_id: &str) -> Result<AttachmentTransfer, RuntimeError> {
        self.ledger.find_attachment(attachment_id)?.ok_or_else(|| {
            gent_ports::LedgerError::Invariant("attachment does not exist".into()).into()
        })
    }

    fn ensure_ingress(&self, transfer: &AttachmentTransfer) -> Result<(), RuntimeError> {
        let ingress = self.ledger.host_ingress()?;
        validate_ingress(
            transfer.host_epoch,
            IngressState {
                epoch: ingress.epoch,
                mode: if ingress.mode == IngressMode::Open {
                    CoreIngressMode::Open
                } else {
                    CoreIngressMode::Closed
                },
            },
        )?;
        Ok(())
    }

    fn require_operation(
        transfer: &AttachmentTransfer,
        operation: &AttachmentOperation,
    ) -> Result<(), RuntimeError> {
        if operation.attachment_id != transfer.metadata.attachment_id
            || operation.transfer_receipt_id != transfer.receipt_id
            || operation.transfer_idempotency_key != transfer.idempotency_key
            || operation.host_epoch != transfer.host_epoch
        {
            return Err(gent_ports::LedgerError::Invariant(
                "attachment operation does not own this transfer".into(),
            )
            .into());
        }
        Ok(())
    }

    fn settle_available_operation(
        &self,
        operation: &AttachmentOperation,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<AttachmentTransfer, RuntimeError> {
        if !claim_receipt(
            &self.ledger,
            &operation.receipt_id,
            &operation.idempotency_key,
            operation.host_epoch,
            kind,
            payload,
        )? {
            return self.resume(&operation.attachment_id);
        }
        settle_receipt(
            &self.ledger,
            &operation.receipt_id,
            &operation.idempotency_key,
            operation.host_epoch,
            kind,
            Err(
                gent_ports::LedgerError::Invariant("attachment is already available".into()).into(),
            ),
        )
    }
}
