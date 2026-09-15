//! Atomic `SQLite` persistence for durable per-turn file checkpoints and their restore.

use gent_ports::{AgentChatCheckpointLedger, LedgerError};
use gent_types::{
    AgentChatCheckpointCapture, AgentChatCheckpointRestore, AgentChatCheckpointRestored,
    AgentChatConversationId, AgentChatFileCheckpoint, AgentChatFileCheckpointFile, AgentChatRunId,
    Command, Receipt, ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;

use super::super::SqliteLedger;
use super::super::queries::{find_receipt, insert_receipt, receipt_matches_command, storage_error};
use super::prompt_dispatch::require_open;

#[path = "checkpoint_reads.rs"]
mod reads;
#[path = "checkpoint_restore.rs"]
mod restore;
use reads::{files_for, find_files, list};
use restore::restore_persist;

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
    require_open(&transaction, capture.host_epoch)?;
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
