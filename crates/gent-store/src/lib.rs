//! `SQLite` implementation of the durable ledger port.

mod attachment_blobs;
mod sqlite;

pub use attachment_blobs::FileAttachmentBlobs;
pub use sqlite::SqliteLedger;

/// Latest `SQLite` migration understood by this build, for signed update compatibility checks.
pub const CURRENT_SCHEMA_VERSION: u32 = 23;
