use gent_ports::LedgerError;
use gent_types::{
    AgentChatConversationId, AgentChatFileCheckpoint, AgentChatFileCheckpointFile, AgentChatRunId,
};
use rusqlite::{OptionalExtension, params};

use super::super::super::SqliteLedger;
use super::super::super::queries::storage_error;

pub(super) fn list(
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

pub(super) fn find_files(
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

pub(super) fn files_for(
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
