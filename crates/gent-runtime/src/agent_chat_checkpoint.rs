//! Authority-gated capture and restore of durable per-turn file checkpoints.

use gent_ports::{AgentChatCheckpointLedger, AttachmentBlobStore};
use gent_types::{
    AgentChatCheckpointCapture, AgentChatCheckpointRestore, AgentChatCheckpointRestored,
    AgentChatFileCheckpoint, AgentChatFileCheckpointFile, AgentChatFileSnapshot, AgentChatRunId,
};
use sha2::{Digest, Sha256};

use crate::RuntimeError;

/// Bytes above this bound are skipped from capture rather than failing the whole checkpoint.
pub const MAX_CHECKPOINT_SNAPSHOT_BYTES: u64 = 2 * 1024 * 1024;
/// Checkpoints retained per conversation; the oldest is evicted beyond this count.
pub const MAX_RETAINED_CHECKPOINTS: usize = 25;

/// Explicit permission to capture or restore durable file checkpoints.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentChatCheckpointAuthority {
    #[default]
    Observer,
    Approved,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatCheckpointCaptureResult {
    DeniedObserver,
    Captured(AgentChatFileCheckpoint),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatCheckpointRestoreResult {
    DeniedObserver,
    Restored(AgentChatCheckpointRestored),
}

/// Orchestrates checkpoint metadata and opaque blob content over durable ports.
#[derive(Clone, Debug)]
pub struct AgentChatCheckpointService<L, B> {
    ledger: L,
    blobs: B,
    authority: AgentChatCheckpointAuthority,
}

impl<L, B> AgentChatCheckpointService<L, B> {
    #[must_use]
    pub fn new(ledger: L, blobs: B, authority: AgentChatCheckpointAuthority) -> Self {
        Self {
            ledger,
            blobs,
            authority,
        }
    }
}

impl<L: AgentChatCheckpointLedger, B: AttachmentBlobStore> AgentChatCheckpointService<L, B> {
    /// Stages each snapshot's bytes as content-addressed blobs, then persists the checkpoint.
    ///
    /// A snapshot over [`MAX_CHECKPOINT_SNAPSHOT_BYTES`] is silently excluded from the captured
    /// file list rather than failing the whole checkpoint, matching the native behavior this
    /// replaces.
    ///
    /// # Errors
    /// Returns an error only after approved authority reaches the durable ledger boundary.
    pub fn capture(
        &self,
        capture: &AgentChatCheckpointCapture,
    ) -> Result<AgentChatCheckpointCaptureResult, RuntimeError> {
        if self.authority != AgentChatCheckpointAuthority::Approved {
            return Ok(AgentChatCheckpointCaptureResult::DeniedObserver);
        }
        let checkpoint_id = stable_identity("checkpoint", &capture.request_id.0);
        let idempotency_key = stable_identity("receipt", &capture.request_id.0);
        let mut files = Vec::with_capacity(capture.files.len());
        for (index, snapshot) in capture.files.iter().enumerate() {
            let bytes = snapshot.content.as_bytes();
            if bytes.len() as u64 > MAX_CHECKPOINT_SNAPSHOT_BYTES {
                continue;
            }
            files.push(stage(&self.blobs, &checkpoint_id, index, snapshot)?);
        }
        let checkpoint = self.ledger.save_file_checkpoint(
            capture,
            &checkpoint_id,
            &idempotency_key,
            &files,
            MAX_RETAINED_CHECKPOINTS,
        )?;
        Ok(AgentChatCheckpointCaptureResult::Captured(checkpoint))
    }

    /// Lists a conversation's checkpoints, most recent first.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    pub fn list(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<AgentChatFileCheckpoint>, RuntimeError> {
        Ok(self.ledger.list_file_checkpoints(conversation_id)?)
    }

    /// Mints the new context boundary and, when `restore_files` is set, writes each captured
    /// blob back to its original path.
    ///
    /// # Errors
    /// Returns an error only after approved authority reaches the durable ledger boundary, or
    /// when a file cannot be written back.
    pub fn restore(
        &self,
        restore: &AgentChatCheckpointRestore,
    ) -> Result<AgentChatCheckpointRestoreResult, RuntimeError> {
        if self.authority != AgentChatCheckpointAuthority::Approved {
            return Ok(AgentChatCheckpointRestoreResult::DeniedObserver);
        }
        let idempotency_key = stable_identity("receipt", &restore.request_id.0);
        let run_id = AgentChatRunId(stable_identity("run", &restore.request_id.0));
        let restored = self
            .ledger
            .restore_file_checkpoint(restore, &idempotency_key, &run_id)?;
        if restore.restore_files {
            for file in &restored.restored_files {
                let bytes = self.blobs.read_attachment_blob(&file.storage_key)?;
                std::fs::write(&file.file_path, bytes).map_err(|error| {
                    gent_ports::LedgerError::Storage(format!(
                        "could not restore {}: {error}",
                        file.file_path
                    ))
                })?;
            }
        }
        Ok(AgentChatCheckpointRestoreResult::Restored(restored))
    }
}

fn stage<B: AttachmentBlobStore>(
    blobs: &B,
    checkpoint_id: &str,
    index: usize,
    snapshot: &AgentChatFileSnapshot,
) -> Result<AgentChatFileCheckpointFile, RuntimeError> {
    let staging_key = format!(
        "staging/{:x}",
        Sha256::digest(format!("gent-checkpoint-v1\0{checkpoint_id}\0{index}").as_bytes())
    );
    let bytes = snapshot.content.as_bytes();
    blobs.append_attachment_chunk(&staging_key, 0, bytes)?;
    let (byte_len, digest) = blobs.attachment_digest(&staging_key, &staging_key)?;
    let storage_key = format!("sha256/{digest}");
    blobs.commit_attachment_blob(&staging_key, &storage_key)?;
    Ok(AgentChatFileCheckpointFile {
        file_path: snapshot.file_path.clone(),
        storage_key,
        byte_len,
    })
}

fn stable_identity(kind: &str, request_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"gent-agent-chat-checkpoint-v1\0");
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(request_id.as_bytes());
    format!("{kind}-{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::{
        AgentChatCheckpointAuthority, AgentChatCheckpointCaptureResult,
        AgentChatCheckpointRestoreResult, AgentChatCheckpointService,
    };
    use gent_ports::{AgentChatWorkspaceLedger, AttachmentLedger};
    use gent_store::{FileAttachmentBlobs, SqliteLedger};
    use gent_types::{
        AgentChatCheckpointCapture, AgentChatCheckpointRestore, AgentChatConversationCreate,
        AgentChatConversationId, AgentChatEffort, AgentChatFileSnapshot, AgentChatMode,
        AgentChatProvider, AgentChatRequestId, AgentChatRunId, AgentChatSelection, HostEpoch,
        ReceiptId, WorkspaceRecord,
    };

    fn service(
        ledger: SqliteLedger,
        root: &std::path::Path,
    ) -> AgentChatCheckpointService<SqliteLedger, FileAttachmentBlobs> {
        AgentChatCheckpointService::new(
            ledger,
            FileAttachmentBlobs::open(root).unwrap(),
            AgentChatCheckpointAuthority::Approved,
        )
    }

    #[test]
    fn observer_authority_denies_capture_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        let service = AgentChatCheckpointService::new(
            SqliteLedger::in_memory().unwrap(),
            FileAttachmentBlobs::open(dir.path()).unwrap(),
            AgentChatCheckpointAuthority::Observer,
        );
        let captured = service
            .capture(&AgentChatCheckpointCapture {
                request_id: AgentChatRequestId("capture-1".into()),
                receipt_id: ReceiptId("receipt-1".into()),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
                message_ordinal: 1,
                created_at_unix_ms: 1,
                files: vec![],
            })
            .unwrap();
        assert_eq!(captured, AgentChatCheckpointCaptureResult::DeniedObserver);
    }

    #[test]
    fn capture_then_restore_writes_the_snapshot_back_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let workspace_file = dir.path().join("main.rs");
        std::fs::write(&workspace_file, b"after edit").unwrap();
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("conversation-receipt".into()),
                    idempotency_key: "conversation-key".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: AgentChatConversationId("conversation-1".into()),
                    run_id: AgentChatRunId("run-1".into()),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Claude,
                        model: "claude-sonnet".into(),
                        effort: AgentChatEffort::Medium,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-1".into(),
                    canonical_path: dir.path().to_string_lossy().into_owned(),
                },
            )
            .unwrap();
        let blobs_root = dir.path().join("blobs-root");
        let service = service(ledger, &blobs_root);
        let captured = service
            .capture(&AgentChatCheckpointCapture {
                request_id: AgentChatRequestId("capture-1".into()),
                receipt_id: ReceiptId("capture-receipt".into()),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
                message_ordinal: 1,
                created_at_unix_ms: 1000,
                files: vec![AgentChatFileSnapshot {
                    file_path: workspace_file.to_string_lossy().into_owned(),
                    content: "before edit".into(),
                }],
            })
            .unwrap();
        let AgentChatCheckpointCaptureResult::Captured(checkpoint) = captured else {
            unreachable!()
        };
        assert_eq!(checkpoint.files.len(), 1);
        assert_eq!(service.list("conversation-1").unwrap().len(), 1);

        std::fs::write(&workspace_file, b"an even later edit").unwrap();

        let restored = service
            .restore(&AgentChatCheckpointRestore {
                request_id: AgentChatRequestId("restore-1".into()),
                receipt_id: ReceiptId("restore-receipt".into()),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                checkpoint_id: checkpoint.checkpoint_id.clone(),
                restore_files: true,
                restore_files_confirmation: Some(checkpoint.checkpoint_id.clone()),
            })
            .unwrap();
        let AgentChatCheckpointRestoreResult::Restored(restored) = restored else {
            unreachable!()
        };
        assert_eq!(restored.visible_through_ordinal, 1);
        assert_eq!(
            std::fs::read_to_string(&workspace_file).unwrap(),
            "before edit"
        );
    }

    #[test]
    fn restoring_files_without_confirmation_is_rejected_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("conversation-receipt".into()),
                    idempotency_key: "conversation-key".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: AgentChatConversationId("conversation-1".into()),
                    run_id: AgentChatRunId("run-1".into()),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Claude,
                        model: "claude-sonnet".into(),
                        effort: AgentChatEffort::Medium,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-1".into(),
                    canonical_path: "/workspace-1".into(),
                },
            )
            .unwrap();
        let blobs_root = dir.path().join("blobs-root");
        let service = service(ledger, &blobs_root);
        let captured = service
            .capture(&AgentChatCheckpointCapture {
                request_id: AgentChatRequestId("capture-1".into()),
                receipt_id: ReceiptId("capture-receipt".into()),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
                message_ordinal: 1,
                created_at_unix_ms: 1000,
                files: vec![],
            })
            .unwrap();
        let AgentChatCheckpointCaptureResult::Captured(checkpoint) = captured else {
            unreachable!()
        };
        let rejected = service.restore(&AgentChatCheckpointRestore {
            request_id: AgentChatRequestId("restore-1".into()),
            receipt_id: ReceiptId("restore-receipt".into()),
            host_epoch: HostEpoch(1),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            checkpoint_id: checkpoint.checkpoint_id,
            restore_files: true,
            restore_files_confirmation: None,
        });
        assert!(rejected.is_err());
    }

    #[test]
    fn oversized_snapshot_is_excluded_but_the_checkpoint_still_captures() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = SqliteLedger::in_memory().unwrap();
        ledger
            .create_agent_chat_conversation_in_workspace(
                &AgentChatConversationCreate {
                    receipt_id: ReceiptId("conversation-receipt".into()),
                    idempotency_key: "conversation-key".into(),
                    host_epoch: HostEpoch(1),
                    conversation_id: AgentChatConversationId("conversation-1".into()),
                    run_id: AgentChatRunId("run-1".into()),
                    selection: AgentChatSelection {
                        provider: AgentChatProvider::Claude,
                        model: "claude-sonnet".into(),
                        effort: AgentChatEffort::Medium,
                        mode: AgentChatMode::Agent,
                    },
                },
                &WorkspaceRecord {
                    workspace_id: "workspace-1".into(),
                    canonical_path: "/workspace-1".into(),
                },
            )
            .unwrap();
        let blobs_root = dir.path().join("blobs-root");
        let service = service(ledger, &blobs_root);
        let huge = "a".repeat(3 * 1024 * 1024);
        let captured = service
            .capture(&AgentChatCheckpointCapture {
                request_id: AgentChatRequestId("capture-1".into()),
                receipt_id: ReceiptId("capture-receipt".into()),
                host_epoch: HostEpoch(1),
                conversation_id: AgentChatConversationId("conversation-1".into()),
                run_id: AgentChatRunId("run-1".into()),
                message_ordinal: 1,
                created_at_unix_ms: 1000,
                files: vec![
                    AgentChatFileSnapshot {
                        file_path: "huge.rs".into(),
                        content: huge,
                    },
                    AgentChatFileSnapshot {
                        file_path: "small.rs".into(),
                        content: "fits".into(),
                    },
                ],
            })
            .unwrap();
        let AgentChatCheckpointCaptureResult::Captured(checkpoint) = captured else {
            unreachable!()
        };
        assert_eq!(checkpoint.files.len(), 1);
        assert_eq!(checkpoint.files[0].file_path, "small.rs");
    }
}
