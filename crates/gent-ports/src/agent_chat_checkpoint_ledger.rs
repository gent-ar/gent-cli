//! Durable boundary for per-conversation file-checkpoint metadata.

use gent_types::{
    AgentChatCheckpointCapture, AgentChatCheckpointRestore, AgentChatCheckpointRestored,
    AgentChatFileCheckpoint, AgentChatFileCheckpointFile, AgentChatRunId,
};

use crate::LedgerError;

/// Persistence boundary for immutable file-checkpoint metadata rows.
///
/// Blob bytes live in the shared `AttachmentBlobStore`, keyed by `storage_key`; this ledger only
/// ever stores metadata (`file_path`, `storage_key`, `byte_len`) and eviction bookkeeping.
pub trait AgentChatCheckpointLedger: Send + Sync {
    /// Persists one immutable checkpoint's already-staged file list under a receipt-backed,
    /// idempotency-safe write, evicting the oldest checkpoint beyond `max_retained` for the same
    /// conversation.
    ///
    /// # Errors
    /// Returns an error when identities are invalid, the idempotency key is owned by another
    /// command, or the write cannot persist.
    fn save_file_checkpoint(
        &self,
        capture: &AgentChatCheckpointCapture,
        checkpoint_id: &str,
        idempotency_key: &str,
        files: &[AgentChatFileCheckpointFile],
        max_retained: usize,
    ) -> Result<AgentChatFileCheckpoint, LedgerError>;

    /// Lists a conversation's checkpoints, most recent first.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn list_file_checkpoints(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<AgentChatFileCheckpoint>, LedgerError>;

    /// Reads one checkpoint's file list by id, scoped to its owning conversation.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_file_checkpoint(
        &self,
        conversation_id: &str,
        checkpoint_id: &str,
    ) -> Result<Option<Vec<AgentChatFileCheckpointFile>>, LedgerError>;

    /// Atomically mints a new child run whose context resumes at the checkpoint's ordinal.
    ///
    /// # Errors
    /// Returns an error when the checkpoint does not belong to the conversation, the restore
    /// confirmation is missing while `restore_files` is set, or the write cannot persist.
    fn restore_file_checkpoint(
        &self,
        restore: &AgentChatCheckpointRestore,
        idempotency_key: &str,
        run_id: &AgentChatRunId,
    ) -> Result<AgentChatCheckpointRestored, LedgerError>;
}
