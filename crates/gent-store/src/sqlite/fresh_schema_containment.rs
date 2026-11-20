use super::{has_column, has_table, has_tables};
use crate::sqlite::queries::storage_error;
use gent_ports::LedgerError;

pub(super) fn add_conversation_containment(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), LedgerError> {
    if !has_table(transaction, "agent_chat_conversation_configs")?
        || has_column(
            transaction,
            "agent_chat_conversation_configs",
            "containment",
        )?
    {
        return Ok(());
    }
    transaction
        .execute(
            "ALTER TABLE agent_chat_conversation_configs ADD COLUMN containment TEXT NOT NULL DEFAULT 'unsandboxed' CHECK (containment IN ('unsandboxed', 'sandboxed'))",
            [],
        )
        .map(|_| ())
        .map_err(storage_error)
}

fn add_policy_command_grants(transaction: &rusqlite::Transaction<'_>) -> Result<(), LedgerError> {
    if !has_table(transaction, "policies")?
        || has_column(transaction, "policies", "allowed_commands")?
    {
        return Ok(());
    }
    transaction
        .execute(
            "ALTER TABLE policies ADD COLUMN allowed_commands TEXT NOT NULL DEFAULT '[]'",
            [],
        )
        .map(|_| ())
        .map_err(storage_error)
}

fn add_side_question_failure_code(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), LedgerError> {
    if !has_table(transaction, "agent_chat_side_questions")?
        || has_column(transaction, "agent_chat_side_questions", "failure_code")?
    {
        return Ok(());
    }
    transaction
        .execute_batch(
            "ALTER TABLE agent_chat_side_questions ADD COLUMN failure_code TEXT CHECK (failure_code IN ('providerBusy', 'providerUnavailable', 'providerFailed'));
             UPDATE agent_chat_side_questions SET failure_code = 'providerFailed' WHERE status = 'failed';",
        )
        .map_err(storage_error)
}

pub(super) fn reapply_idempotent_schema(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), LedgerError> {
    add_policy_command_grants(transaction)?;
    add_side_question_failure_code(transaction)?;
    transaction
        .execute_batch(include_str!("fresh_schema_archive.sql"))
        .map_err(storage_error)?;
    if has_table(transaction, "run_lifecycle_facts")? {
        transaction
            .execute_batch(include_str!("fresh_schema_lifecycle_status.sql"))
            .map_err(storage_error)?;
    }
    if !has_tables(
        transaction,
        &[
            "events",
            "agent_chat_conversations",
            "agent_chat_projection_events",
            "turns",
        ],
    )? {
        return Ok(());
    }
    transaction
        .execute_batch(concat!(
            include_str!("fresh_schema_permission_projection.sql"),
            include_str!("fresh_schema_turn_boundaries.sql")
        ))
        .map_err(storage_error)
}
