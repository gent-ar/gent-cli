//! Durable, secret-free checkpoints for ordered run recovery metadata.

use serde::{Deserialize, Serialize};

/// An immutable checkpoint for one durable run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCheckpointRecord {
    pub checkpoint_id: String,
    pub run_id: String,
    /// Strictly increasing sequence scoped to one run.
    pub sequence: u64,
    /// Durable event head covered by the checkpoint.
    pub event_cursor: u64,
    /// SHA-256 of opaque checkpoint material held by the owning execution boundary.
    pub state_digest_sha256: String,
}
