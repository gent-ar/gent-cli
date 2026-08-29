//! Atomic `SQLite` persistence for durable per-turn file checkpoints and their restore.

use gent_ports::{AgentChatCheckpointLedger, IngressMode, LedgerError};
use gent_types::{
    AgentChatCheckpointCapture, AgentChatCheckpointRestore, AgentChatCheckpointRestored,
    AgentChatConversationId, AgentChatFileCheckpoint, AgentChatFileCheckpointFile, AgentChatRunId,
    Command, Receipt, ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;

use super::super::SqliteLedger;
use super::super::epoch::require_epoch;
use super::super::queries::{
    find_receipt, host_ingress, insert_receipt, receipt_matches_command, storage_error,
};

impl AgentChatCheckpointLedger for SqliteLedger {
    fn save_file_checkpoint(
        &self,
        capture: &AgentChatCheckpointCapture,
        checkpoint_id: &str,
        idempotency_key: &str,
        files: &[AgentChatFileCheckpointFile],
        max_retained: usize,
    ) -> Result<AgentChatFileCheckpoint, LedgerError> {
        save(
            self,
            capture,
            checkpoint_id,
            idempotency_key,
            files,
            max_retained,
        )
    }

    fn list_file_checkpoints(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<AgentChatFileCheckpoint>, LedgerError> {
        list(self, conversation_id)
    }

    fn find_file_checkpoint(
        &self,
        conversation_id: &str,
        checkpoint_id: &str,
    ) -> Result<Option<Vec<AgentChatFileCheckpointFile>>, LedgerError> {
        find_files(self, conversation_id, checkpoint_id)
    }

    fn restore_file_checkpoint(
        &self,
        restore: &AgentChatCheckpointRestore,
        idempotency_key: &str,
        run_id: &AgentChatRunId,
    ) -> Result<AgentChatCheckpointRestored, LedgerError> {
        restore_persist(self, restore, idempotency_key, run_id)
    }
}

fn save(
    ledger: &SqliteLedger,
    capture: &AgentChatCheckpointCapture,
    checkpoint_id: &str,
    idempotency_key: &str,
    files: &[AgentChatFileCheckpointFile],
    max_retained: usize,
) -> Result<AgentChatFileCheckpoint, LedgerError> {
    validate_capture(capture, checkpoint_id)?;
    let command = capture_command(capture, checkpoint_id, idempotency_key);
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(capture.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    if let Some(existing) = existing_capture(&transaction, idempotency_key, capture, checkpoint_id)?
    {
        if !receipt_matches_command(&transaction, &command)? {
            return Err(capture_conflict());
        }
        return Ok(existing);
    }
    if find_receipt(&transaction, idempotency_key)?.is_some() {
        return Err(LedgerError::Invariant(
            "agent chat checkpoint idempotency key is owned by another command".into(),
        ));
    }
    let receipt = Receipt {
        receipt_id: capture.receipt_id.clone(),
        idempotency_key: idempotency_key.to_owned(),
        status: ReceiptStatus::Settled,
        host_epoch: capture.host_epoch,
    };
    insert_receipt(&transaction, &receipt, &command)?;
    transaction
        .execute(
            "INSERT INTO agent_chat_file_checkpoints (checkpoint_id, idempotency_key, conversation_id, run_id, message_ordinal, created_at_unix_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                checkpoint_id,
                idempotency_key,
                capture.conversation_id.0,
                capture.run_id.0,
                capture.message_ordinal,
                capture.created_at_unix_ms,
            ],
        )
        .map_err(storage_error)?;
    for file in files {
        transaction
            .execute(
                "INSERT INTO agent_chat_checkpoint_files (checkpoint_id, file_path, storage_key, byte_len) VALUES (?1, ?2, ?3, ?4)",
                params![checkpoint_id, file.file_path, file.storage_key, file.byte_len],
            )
            .map_err(storage_error)?;
    }
    evict_oldest(&transaction, &capture.conversation_id.0, max_retained)?;
    transaction.commit().map_err(storage_error)?;
    Ok(AgentChatFileCheckpoint {
        checkpoint_id: checkpoint_id.to_owned(),
        conversation_id: capture.conversation_id.clone(),
        run_id: capture.run_id.clone(),
        message_ordinal: capture.message_ordinal,
        created_at_unix_ms: capture.created_at_unix_ms,
        files: files.to_vec(),
    })
}

fn validate_capture(
    capture: &AgentChatCheckpointCapture,
    checkpoint_id: &str,
) -> Result<(), LedgerError> {
    if checkpoint_id.trim().is_empty()
        || capture.receipt_id.0.trim().is_empty()
        || capture.request_id.0.trim().is_empty()
        || capture.conversation_id.0.trim().is_empty()
        || capture.run_id.0.trim().is_empty()
    {
        return Err(LedgerError::Invariant(
            "agent chat checkpoint identities must be nonempty".into(),
        ));
    }
    Ok(())
}

fn capture_conflict() -> LedgerError {
    LedgerError::Invariant(
        "agent chat checkpoint capture retry conflicts with durable ownership".into(),
    )
}

fn existing_capture(
    transaction: &Transaction<'_>,
    idempotency_key: &str,
    capture: &AgentChatCheckpointCapture,
    checkpoint_id: &str,
) -> Result<Option<AgentChatFileCheckpoint>, LedgerError> {
    let row = transaction
        .query_row(
            "SELECT checkpoint_id, conversation_id, run_id, message_ordinal, created_at_unix_ms FROM agent_chat_file_checkpoints WHERE idempotency_key = ?1",
            [idempotency_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u64>(3)?,
                    row.get::<_, u64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    let Some((existing_id, conversation, run, ordinal, created_at)) = row else {
        return Ok(None);
    };
    if existing_id != checkpoint_id
        || conversation != capture.conversation_id.0
        || run != capture.run_id.0
    {
        return Err(capture_conflict());
    }
    let files = files_for(transaction, &existing_id)?;
    Ok(Some(AgentChatFileCheckpoint {
        checkpoint_id: existing_id,
        conversation_id: AgentChatConversationId(conversation),
        run_id: AgentChatRunId(run),
        message_ordinal: ordinal,
        created_at_unix_ms: created_at,
        files,
    }))
}

fn capture_command(
    capture: &AgentChatCheckpointCapture,
    checkpoint_id: &str,
    idempotency_key: &str,
) -> Command {
    Command {
        receipt_id: capture.receipt_id.clone(),
        idempotency_key: idempotency_key.to_owned(),
        host_epoch: capture.host_epoch,
        kind: "agentChatCaptureCheckpoint".into(),
        payload: json!({
            "checkpointId": checkpoint_id,
            "conversationId": capture.conversation_id.0,
            "runId": capture.run_id.0,
            "messageOrdinal": capture.message_ordinal,
        }),
    }
}

fn evict_oldest(
    transaction: &Transaction<'_>,
    conversation_id: &str,
    max_retained: usize,
) -> Result<(), LedgerError> {
    let max_retained = i64::try_from(max_retained)
        .map_err(|_| LedgerError::Invariant("checkpoint retention limit overflow".into()))?;
    let stale: Vec<String> = {
        let mut statement = transaction
            .prepare(
                "SELECT checkpoint_id FROM agent_chat_file_checkpoints WHERE conversation_id = ?1 ORDER BY created_at_unix_ms DESC, checkpoint_id DESC LIMIT -1 OFFSET ?2",
            )
            .map_err(storage_error)?;
        statement
            .query_map(params![conversation_id, max_retained], |row| row.get(0))
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?
    };
    for checkpoint_id in stale {
        transaction
            .execute(
                "DELETE FROM agent_chat_checkpoint_files WHERE checkpoint_id = ?1",
                [&checkpoint_id],
            )
            .map_err(storage_error)?;
        transaction
            .execute(
                "DELETE FROM agent_chat_file_checkpoints WHERE checkpoint_id = ?1",
                [&checkpoint_id],
            )
            .map_err(storage_error)?;
    }
    Ok(())
}

fn list(
    ledger: &SqliteLedger,
    conversation_id: &str,
) -> Result<Vec<AgentChatFileCheckpoint>, LedgerError> {
    let connection = ledger.lock()?;
    let mut statement = connection
        .prepare(
            "SELECT checkpoint_id, run_id, message_ordinal, created_at_unix_ms FROM agent_chat_file_checkpoints WHERE conversation_id = ?1 ORDER BY created_at_unix_ms DESC, checkpoint_id DESC",
        )
        .map_err(storage_error)?;
    let heads = statement
        .query_map([conversation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, u64>(3)?,
            ))
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    let mut checkpoints = Vec::with_capacity(heads.len());
    for (checkpoint_id, run_id, message_ordinal, created_at_unix_ms) in heads {
        let files = files_for(&connection, &checkpoint_id)?;
        checkpoints.push(AgentChatFileCheckpoint {
            checkpoint_id,
            conversation_id: AgentChatConversationId(conversation_id.to_owned()),
            run_id: AgentChatRunId(run_id),
            message_ordinal,
            created_at_unix_ms,
            files,
        });
    }
    Ok(checkpoints)
}

fn find_files(
    ledger: &SqliteLedger,
    conversation_id: &str,
    checkpoint_id: &str,
) -> Result<Option<Vec<AgentChatFileCheckpointFile>>, LedgerError> {
    let connection = ledger.lock()?;
    let owned = connection
        .query_row(
            "SELECT 1 FROM agent_chat_file_checkpoints WHERE checkpoint_id = ?1 AND conversation_id = ?2",
            params![checkpoint_id, conversation_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?;
    if owned.is_none() {
        return Ok(None);
    }
    Ok(Some(files_for(&connection, checkpoint_id)?))
}

fn files_for(
    connection: &rusqlite::Connection,
    checkpoint_id: &str,
) -> Result<Vec<AgentChatFileCheckpointFile>, LedgerError> {
    let mut statement = connection
        .prepare(
            "SELECT file_path, storage_key, byte_len FROM agent_chat_checkpoint_files WHERE checkpoint_id = ?1 ORDER BY file_path ASC",
        )
        .map_err(storage_error)?;
    statement
        .query_map([checkpoint_id], |row| {
            Ok(AgentChatFileCheckpointFile {
                file_path: row.get(0)?,
                storage_key: row.get(1)?,
                byte_len: row.get(2)?,
            })
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)
}

fn restore_persist(
    ledger: &SqliteLedger,
    restore: &AgentChatCheckpointRestore,
    idempotency_key: &str,
    run_id: &AgentChatRunId,
) -> Result<AgentChatCheckpointRestored, LedgerError> {
    validate_restore(restore)?;
    let command = command_for(restore, idempotency_key, run_id);
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(restore.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    if let Some(result) = existing_restore(&transaction, idempotency_key, restore)? {
        if !receipt_matches_command(&transaction, &command)? {
            return Err(conflict());
        }
        return Ok(result);
    }
    if find_receipt(&transaction, idempotency_key)?.is_some() {
        return Err(LedgerError::Invariant(
            "agent chat checkpoint restore idempotency key is owned by another command".into(),
        ));
    }
    let message_ordinal = transaction
        .query_row(
            "SELECT message_ordinal FROM agent_chat_file_checkpoints WHERE checkpoint_id = ?1 AND conversation_id = ?2",
            params![restore.checkpoint_id, restore.conversation_id.0],
            |row| row.get::<_, u64>(0),
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| {
            LedgerError::Invariant(
                "agent chat checkpoint does not belong to the restoring conversation".into(),
            )
        })?;
    let (provider, model, effort, mode) =
        current_selection(&transaction, &restore.conversation_id.0)?;
    transaction
        .execute(
            "INSERT INTO runs (run_id, conversation_id, parent_run_id, provider) VALUES (?1, ?2, NULL, ?3)",
            params![run_id.0, restore.conversation_id.0, provider],
        )
        .map_err(storage_error)?;
    transaction.execute("INSERT INTO agent_chat_run_selections (run_id, provider, model, effort, mode) VALUES (?1, ?2, ?3, ?4, ?5)", params![run_id.0, provider, model, effort, mode]).map_err(storage_error)?;
    let receipt = Receipt {
        receipt_id: restore.receipt_id.clone(),
        idempotency_key: idempotency_key.to_owned(),
        status: ReceiptStatus::Settled,
        host_epoch: restore.host_epoch,
    };
    insert_receipt(&transaction, &receipt, &command)?;
    transaction
        .execute(
            "INSERT INTO agent_chat_checkpoint_restore_receipts (idempotency_key, conversation_id, checkpoint_id, run_id, visible_through_ordinal) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![idempotency_key, restore.conversation_id.0, restore.checkpoint_id, run_id.0, message_ordinal],
        )
        .map_err(storage_error)?;
    let restored_files = files_for(&transaction, &restore.checkpoint_id)?;
    transaction.commit().map_err(storage_error)?;
    Ok(AgentChatCheckpointRestored {
        receipt,
        conversation_id: restore.conversation_id.clone(),
        checkpoint_id: restore.checkpoint_id.clone(),
        run_id: run_id.clone(),
        visible_through_ordinal: message_ordinal,
        restored_files,
    })
}

fn validate_restore(restore: &AgentChatCheckpointRestore) -> Result<(), LedgerError> {
    if restore.receipt_id.0.trim().is_empty()
        || restore.request_id.0.trim().is_empty()
        || restore.conversation_id.0.trim().is_empty()
        || restore.checkpoint_id.trim().is_empty()
    {
        return Err(LedgerError::Invariant(
            "agent chat checkpoint restore identities must be nonempty".into(),
        ));
    }
    if restore.restore_files
        && restore
            .restore_files_confirmation
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Err(LedgerError::Invariant(
            "restoring files requires an explicit non-empty confirmation".into(),
        ));
    }
    Ok(())
}

fn conflict() -> LedgerError {
    LedgerError::Invariant(
        "agent chat checkpoint restore retry conflicts with durable ownership".into(),
    )
}

fn existing_restore(
    transaction: &Transaction<'_>,
    idempotency_key: &str,
    restore: &AgentChatCheckpointRestore,
) -> Result<Option<AgentChatCheckpointRestored>, LedgerError> {
    let row = transaction
        .query_row(
            "SELECT r.receipt_id, r.status, r.host_epoch, c.conversation_id, c.checkpoint_id, c.run_id, c.visible_through_ordinal FROM agent_chat_checkpoint_restore_receipts c JOIN receipts r ON r.idempotency_key = c.idempotency_key WHERE c.idempotency_key = ?1",
            [idempotency_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, u64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    let Some((receipt_id, status, epoch, conversation, checkpoint_id, run, ordinal)) = row else {
        return Ok(None);
    };
    if receipt_id != restore.receipt_id.0
        || conversation != restore.conversation_id.0
        || checkpoint_id != restore.checkpoint_id
        || status != "settled"
    {
        return Err(conflict());
    }
    let restored_files = files_for(transaction, &checkpoint_id)?;
    Ok(Some(AgentChatCheckpointRestored {
        receipt: Receipt {
            receipt_id: restore.receipt_id.clone(),
            idempotency_key: idempotency_key.to_owned(),
            status: ReceiptStatus::Settled,
            host_epoch: gent_types::HostEpoch(epoch),
        },
        conversation_id: restore.conversation_id.clone(),
        checkpoint_id,
        run_id: AgentChatRunId(run),
        visible_through_ordinal: ordinal,
        restored_files,
    }))
}

fn current_selection(
    transaction: &Transaction<'_>,
    conversation_id: &str,
) -> Result<(String, String, String, String), LedgerError> {
    transaction
        .query_row(
            "SELECT q.provider, q.model, q.effort, q.mode FROM agent_chat_conversations c JOIN agent_chat_run_selections q JOIN runs r ON r.run_id = q.run_id WHERE c.conversation_id = ?1 AND r.conversation_id = c.conversation_id ORDER BY r.rowid DESC LIMIT 1",
            [conversation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| {
            LedgerError::Invariant("agent chat checkpoint restore conversation is unknown".into())
        })
}

fn command_for(
    restore: &AgentChatCheckpointRestore,
    idempotency_key: &str,
    run_id: &AgentChatRunId,
) -> Command {
    Command {
        receipt_id: restore.receipt_id.clone(),
        idempotency_key: idempotency_key.to_owned(),
        host_epoch: restore.host_epoch,
        kind: "agentChatRestoreCheckpoint".into(),
        payload: json!({
            "conversationId": restore.conversation_id.0,
            "checkpointId": restore.checkpoint_id,
            "runId": run_id.0,
            "restoreFiles": restore.restore_files,
        }),
    }
}
