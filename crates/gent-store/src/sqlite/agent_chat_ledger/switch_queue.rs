use gent_ports::LedgerError;
use rusqlite::{Transaction, params};

use super::super::queries::storage_error;

pub(super) fn carry_undispatched_prompts(transaction: &Transaction<'_>) -> Result<(), LedgerError> {
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
        move_prompt(transaction, &message_id, &turn_id, &run_id)?;
    }
    Ok(())
}

pub(super) fn carry_to_selected_run(
    transaction: &Transaction<'_>,
    run_id: &str,
) -> Result<(), LedgerError> {
    carry_undispatched_prompts(transaction)?;
    transaction
        .execute(
            "UPDATE agent_chat_prompt_dispatches SET state = 'awaiting_readiness' WHERE state = 'pending' AND message_id IN (SELECT message_id FROM conversation_messages WHERE run_id = ?1) AND NOT EXISTS (SELECT 1 FROM agent_chat_prompt_dispatches running JOIN conversation_messages m ON m.message_id = running.message_id WHERE m.conversation_id = (SELECT conversation_id FROM runs WHERE run_id = ?1) AND running.state IN ('claimed', 'launching', 'started'))",
            [run_id],
        )
        .map_err(storage_error)?;
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
