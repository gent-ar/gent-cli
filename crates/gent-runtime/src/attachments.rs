//! Attachment transfer orchestration over durable metadata and opaque blob ports.

use gent_core::{
    IngressMode as CoreIngressMode, IngressState, accept_chunk, commit, validate_attachment,
    validate_ingress, validate_staging_key,
};
use gent_ports::{AttachmentBlobStore, AttachmentClaim, AttachmentLedger, IngressMode, Ledger};
use gent_types::{AttachmentState, AttachmentTransfer};

use crate::RuntimeError;

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
        match self.ledger.claim_attachment(transfer)? {
            AttachmentClaim::Created(value) | AttachmentClaim::Existing(value) => Ok(value),
        }
    }

    /// Appends one validated sequential chunk; retries are safe when a prior blob write succeeded first.
    ///
    /// # Errors
    /// Returns an error for stale ingress, missing metadata, or invalid ordered progress.
    pub fn append(
        &self,
        attachment_id: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<AttachmentTransfer, RuntimeError> {
        let current = self.ledger.find_attachment(attachment_id)?.ok_or_else(|| {
            gent_ports::LedgerError::Invariant("attachment does not exist".into())
        })?;
        if current.state == AttachmentState::Available {
            return Ok(current);
        }
        self.ensure_ingress(&current)?;
        let byte_len = u64::try_from(bytes.len())
            .map_err(|_| gent_ports::LedgerError::Invariant("chunk length overflow".into()))?;
        if offset.checked_add(byte_len) == Some(current.received_bytes) {
            self.blobs
                .append_attachment_chunk(&current.staging_key, offset, bytes)?;
            return Ok(current);
        }
        let next = accept_chunk(&current, offset, byte_len)
            .map_err(|error| gent_ports::LedgerError::Invariant(error.to_string()))?;
        self.blobs
            .append_attachment_chunk(&current.staging_key, offset, bytes)?;
        self.ledger.replace_attachment(&current, &next)?;
        Ok(next)
    }

    /// Verifies the complete staged digest, promotes content, then durably marks it available.
    ///
    /// # Errors
    /// Returns an error without availability when the byte count or digest differs.
    pub fn commit(&self, attachment_id: &str) -> Result<AttachmentTransfer, RuntimeError> {
        let current = self.ledger.find_attachment(attachment_id)?.ok_or_else(|| {
            gent_ports::LedgerError::Invariant("attachment does not exist".into())
        })?;
        if current.state == AttachmentState::Available {
            return Ok(current);
        }
        self.ensure_ingress(&current)?;
        let (size, digest) = self
            .blobs
            .attachment_digest(&current.staging_key, &current.metadata.storage_key)?;
        if size != current.received_bytes {
            return Err(gent_ports::LedgerError::Invariant(
                "staged attachment size differs from durable progress".into(),
            )
            .into());
        }
        let next = commit(&current, &digest)
            .map_err(|error| gent_ports::LedgerError::Invariant(error.to_string()))?;
        self.blobs
            .commit_attachment_blob(&current.staging_key, &current.metadata.storage_key)?;
        self.ledger.replace_attachment(&current, &next)?;
        Ok(next)
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
}
