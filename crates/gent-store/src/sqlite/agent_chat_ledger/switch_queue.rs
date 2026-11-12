use gent_ports::LedgerError;
use gent_types::HostEpoch;
use rusqlite::{Transaction, params};

use super::super::agent_chat_steer_target as target;
use super::super::queries::storage_error;

pub(super) fn carry_undispatched_prompts(
    transaction: &Transaction<'_>,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    let stranded = transaction
        .prepare("SELECT m.message_id, m.turn_id, (SELECT latest.run_id FROM runs latest JOIN agent_chat_run_selections selected ON selected.run_id = latest.run_id WHERE latest.conversation_id = m.conversation_id ORDER BY latest.rowid DESC LIMIT 1) AS current_run_id FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id WHERE d.state IN ('pending', 'awaiting_readiness') AND m.run_id <> current_run_id ORDER BY d.created_rowid")
        .map_err(storage_error)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    for (message_id, turn_id, run_id) in stranded {
        release_steer_target(transaction, host_epoch, &message_id)?;
        move_prompt(transaction, &message_id, &turn_id, &run_id)?;
    }
    Ok(())
}

pub(super) fn move_prompt(
    transaction: &Transaction<'_>,
    message_id: &str,
    turn_id: &str,
    run_id: &str,
) -> Result<(), LedgerError> {
    transaction
        .execute(
            "UPDATE turns SET run_id = ?1, sequence = (SELECT COALESCE(MAX(sequence), 0) + 1 FROM turns WHERE run_id = ?1) WHERE turn_id = ?2",
            params![run_id, turn_id],
        )
        .map_err(storage_error)?;
    for table in ["conversation_messages", "agent_chat_prompt_receipts"] {
        transaction
            .execute(
                &format!("UPDATE {table} SET run_id = ?1 WHERE message_id = ?2"),
                params![run_id, message_id],
            )
            .map_err(storage_error)?;
    }
    Ok(())
}

fn release_steer_target(
    transaction: &Transaction<'_>,
    host_epoch: HostEpoch,
    message_id: &str,
) -> Result<(), LedgerError> {
    if !matches!(target::target(transaction, message_id)?, Some((_, _, _, state)) if state == "requested")
    {
        return Ok(());
    }
    target::expire(transaction, message_id, host_epoch, "selectionSwitched")?;
    transaction
        .execute(
            "DELETE FROM agent_chat_steer_targets WHERE message_id = ?1",
            [message_id],
        )
        .map_err(storage_error)
        .map(|_| ())
}
