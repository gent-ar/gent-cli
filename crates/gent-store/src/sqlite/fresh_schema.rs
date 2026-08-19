//! Fresh-only `SQLite` schema creation for the unreleased Gent ledger.

use gent_ports::LedgerError;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use super::queries::storage_error;

const SCHEMA_ID: &str = "gent-fresh-schema-v7";
const SCHEMA: &str = include_str!("fresh_schema.sql");

/// Opens an empty database with Gent's complete current schema.
///
/// Existing pre-release ledgers are deliberately refused rather than upgraded.
pub(super) fn apply(connection: &mut Connection) -> Result<(), LedgerError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    if let Some(identity) = schema_identity(&transaction)? {
        return (identity == SCHEMA_ID)
            .then_some(())
            .ok_or_else(|| LedgerError::Invariant("unknown Gent fresh schema identity".into()));
    }
    if has_user_tables(&transaction)? {
        return Err(LedgerError::Invariant(
            "existing Gent ledger requires reset; this pre-release build does not migrate data"
                .into(),
        ));
    }
    transaction.execute_batch(SCHEMA).map_err(storage_error)?;
    transaction.commit().map_err(storage_error)
}

fn schema_identity(connection: &Connection) -> Result<Option<String>, LedgerError> {
    let present = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'gent_schema'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?;
    present
        .is_some()
        .then(|| {
            connection
                .query_row(
                    "SELECT identity FROM gent_schema WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )
                .map_err(storage_error)
        })
        .transpose()
}

fn has_user_tables(connection: &Connection) -> Result<bool, LedgerError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(storage_error)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::apply;

    #[test]
    fn rejects_an_existing_ledger_without_attempting_an_upgrade() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE old_gent_ledger (identity TEXT PRIMARY KEY);")
            .unwrap();
        assert!(apply(&mut connection).is_err());
    }

    #[test]
    fn rejects_the_previous_fresh_schema_identity() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
                 INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v4');",
            )
            .unwrap();
        assert!(apply(&mut connection).is_err());
    }
}
