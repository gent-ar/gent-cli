use super::{has_column, has_table};
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
