//! Atomic persistence port for one daemon-normalized provider session fact.

use gent_types::{NormalizedSessionBatch, NormalizedSessionBatchResult};

use crate::LedgerError;

/// Commits lifecycle, transcript, and activity projections as one idempotent durable batch.
///
/// The implementation must enforce host/run/turn ownership and return only after every requested
/// projection plus its source cursor is durable. It must not accept raw provider output.
pub trait NormalizedSessionBatchLedger: Send + Sync {
    /// Persists one exact normalized batch or returns its prior matching result on retry.
    ///
    /// # Errors
    /// Returns an error when identities, ownership, cursors, or retry payloads conflict.
    fn append_normalized_session_batch(
        &self,
        batch: &NormalizedSessionBatch,
    ) -> Result<NormalizedSessionBatchResult, LedgerError>;
}
