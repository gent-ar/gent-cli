use gent_ports::LedgerError;
use gent_types::{
    AgentChatCheckpointRestore, AgentChatCheckpointRestored, AgentChatRunId, Command, Receipt,
    ReceiptStatus,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::json;

use super::super::super::SqliteLedger;
use super::super::super::queries::{
    find_receipt, insert_receipt, receipt_matches_command, storage_error,
};
use super::super::fork::current_selection;
use super::super::prompt_dispatch::require_open;
use super::reads::files_for;

pub(super) fn restore_persist(
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
    require_open(&transaction, restore.host_epoch)?;
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
    let (provider, model, effort, mode, _) =
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
