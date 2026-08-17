//! `SQLite` implementation of the durable ledger port.

mod attachment_blobs;
mod sqlite;

pub use attachment_blobs::FileAttachmentBlobs;
pub use sqlite::SqliteLedger;

/// Fixed fresh-schema contract understood by this build for signed update compatibility checks.
pub const FRESH_SCHEMA_COMPATIBILITY_VERSION: u32 = 1;
