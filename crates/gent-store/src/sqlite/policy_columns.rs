//! Idempotent schema support for the permission-policy revision migration.

use gent_ports::LedgerError;
use rusqlite::{Connection, Transaction};

use super::queries::storage_error;

/// Adds the v24 policy columns when a pre-v24 ledger does not already contain them.
pub(super) fn ensure(transaction: &Transaction<'_>) -> Result<(), LedgerError> {
    ensure_column(transaction, "mode", "TEXT NOT NULL DEFAULT 'default'")?;
    ensure_column(
        transaction,
        "allowed_categories",
        "TEXT NOT NULL DEFAULT '[]'",
    )
}

fn ensure_column(
    transaction: &Transaction<'_>,
    column: &str,
    definition: &str,
) -> Result<(), LedgerError> {
    if !has_column(transaction, column)? {
        transaction
            .execute(
                &format!("ALTER TABLE policies ADD COLUMN {column} {definition}"),
                [],
            )
            .map_err(storage_error)?;
    }
    Ok(())
}

fn has_column(connection: &Connection, column: &str) -> Result<bool, LedgerError> {
    let mut statement = connection
        .prepare("PRAGMA table_info(policies)")
        .map_err(storage_error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(storage_error)?;
    Ok(columns.filter_map(Result::ok).any(|name| name == column))
}
