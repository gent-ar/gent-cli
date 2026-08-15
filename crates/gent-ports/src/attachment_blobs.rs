//! Content-addressed byte staging boundary. It receives no user filesystem paths.

use crate::LedgerError;

/// Atomic content-addressed blob operations used after pure transfer validation.
pub trait AttachmentBlobStore: Send + Sync {
    /// Appends exactly one checked chunk at `offset` to opaque staged content.
    ///
    /// # Errors
    /// Returns an error for an out-of-order write, I/O failure, or unsafe opaque key.
    fn append_attachment_chunk(
        &self,
        storage_key: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), LedgerError>;

    /// Returns the byte count and lowercase SHA-256 digest for staged or committed content.
    ///
    /// # Errors
    /// Returns an error when the staged content does not exist or cannot be read.
    fn attachment_digest(&self, storage_key: &str) -> Result<(u64, String), LedgerError>;

    /// Atomically promotes checked staged content to its content-addressed immutable location.
    ///
    /// # Errors
    /// Returns an error when promotion cannot complete safely.
    fn commit_attachment_blob(&self, storage_key: &str) -> Result<(), LedgerError>;
}
