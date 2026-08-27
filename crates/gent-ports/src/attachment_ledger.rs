//! Durable metadata and turn-association boundary for staged attachments.

use gent_types::{AttachmentMetadata, AttachmentTransfer, TurnAttachment};

use crate::LedgerError;

/// Result of claiming a receipt-scoped attachment transfer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachmentClaim {
    Created(AttachmentTransfer),
    Existing(AttachmentTransfer),
}

/// Persists opaque attachment metadata and progress, never local source paths or bytes.
pub trait AttachmentLedger: Send + Sync {
    /// Creates a transfer or returns the one that already owns its idempotency key.
    ///
    /// # Errors
    /// Returns an error for conflicting immutable metadata or a storage failure.
    fn claim_attachment(
        &self,
        transfer: &AttachmentTransfer,
    ) -> Result<AttachmentClaim, LedgerError>;

    /// Advances progress only from its expected durable byte count and state.
    ///
    /// # Errors
    /// Returns an error for a missing or concurrently changed transfer.
    fn replace_attachment(
        &self,
        expected: &AttachmentTransfer,
        next: &AttachmentTransfer,
    ) -> Result<(), LedgerError>;

    /// Finds metadata and current transfer progress by opaque attachment identifier.
    ///
    /// # Errors
    /// Returns an error when durable storage cannot be read.
    fn find_attachment(
        &self,
        attachment_id: &str,
    ) -> Result<Option<AttachmentTransfer>, LedgerError>;

    /// Associates one available attachment with a durable turn under its active host fence.
    ///
    /// # Errors
    /// Returns an error when either identity is unknown or the association already conflicts.
    fn attach_to_turn(&self, association: &TurnAttachment) -> Result<(), LedgerError>;

    fn turn_attachments(&self, turn_id: &str) -> Result<Vec<AttachmentMetadata>, LedgerError>;
}
