//! `SQLite` implementation of the durable ledger port.

mod attachment_blobs;
mod sqlite;

pub use attachment_blobs::FileAttachmentBlobs;
pub use sqlite::SqliteLedger;

pub const CURRENT_LEDGER_SCHEMA_VERSION: u32 = 29;
pub const FRESH_SCHEMA_COMPATIBILITY_VERSION: u32 = CURRENT_LEDGER_SCHEMA_VERSION;
