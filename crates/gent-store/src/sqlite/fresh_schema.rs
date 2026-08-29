//! Fresh-only `SQLite` schema creation for the unreleased Gent ledger.

use gent_ports::LedgerError;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use super::queries::storage_error;

const SCHEMA_ID: &str = "gent-fresh-schema-v15";
const PREVIOUS_SCHEMA_ID: &str = "gent-fresh-schema-v10";
const SCHEMA: &str = concat!(
    include_str!("fresh_schema.sql"),
    include_str!("fresh_schema_agent_chat.sql")
);
const V10_TO_V15: &str = include_str!("fresh_schema_v10_to_v15.sql");

/// Opens an empty database with Gent's complete current schema.
///
pub(super) fn apply(connection: &mut Connection) -> Result<(), LedgerError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    if let Some(identity) = schema_identity(&transaction)? {
        if identity == SCHEMA_ID {
            return Ok(());
        }
        if identity == PREVIOUS_SCHEMA_ID {
            transaction
                .execute_batch(V10_TO_V15)
                .map_err(storage_error)?;
            if !has_column(
                &transaction,
                "agent_chat_prompt_receipts",
                "tool_source_ids_json",
            )? {
                transaction
                    .execute(
                        "ALTER TABLE agent_chat_prompt_receipts ADD COLUMN tool_source_ids_json TEXT NOT NULL DEFAULT '[]'",
                        [],
                    )
                    .map_err(storage_error)?;
            }
            transaction.commit().map_err(storage_error)?;
            return Ok(());
        }
        return Err(LedgerError::Invariant(
            "unknown Gent fresh schema identity".into(),
        ));
    }
    if has_user_tables(&transaction)? {
        return Err(LedgerError::Invariant(
            "existing Gent ledger requires reset; this build accepts only an empty database".into(),
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

fn has_column(connection: &Connection, table: &str, column: &str) -> Result<bool, LedgerError> {
    connection
        .query_row(
            "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2 LIMIT 1",
            [table, column],
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
    fn rejects_an_existing_ledger_without_altering_data() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE old_gent_ledger (identity TEXT PRIMARY KEY);")
            .unwrap();
        assert!(apply(&mut connection).is_err());
    }

    #[test]
    fn rejects_an_unknown_fresh_schema_identity() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
                 INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v4');",
            )
            .unwrap();
        assert!(apply(&mut connection).is_err());
    }

    #[test]
    fn migrates_the_previous_fresh_schema_identity() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE gent_schema (singleton INTEGER PRIMARY KEY, identity TEXT NOT NULL); \
                 INSERT INTO gent_schema (singleton, identity) VALUES (1, 'gent-fresh-schema-v10'); \
                 CREATE TABLE conversations (conversation_id TEXT PRIMARY KEY NOT NULL); \
                 CREATE TABLE workspaces (workspace_id TEXT PRIMARY KEY NOT NULL); \
                 CREATE TABLE runs (run_id TEXT PRIMARY KEY NOT NULL); \
                 CREATE TABLE receipts (idempotency_key TEXT PRIMARY KEY NOT NULL); \
                 CREATE TABLE mcp_connectors (connector_id TEXT PRIMARY KEY NOT NULL); \
                 CREATE TABLE tool_sources (tool_source_id TEXT PRIMARY KEY NOT NULL); \
                 CREATE TABLE agent_chat_conversations (conversation_id TEXT PRIMARY KEY NOT NULL, root_run_id TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL, effort TEXT NOT NULL, mode TEXT NOT NULL, workspace_id TEXT); \
                 CREATE TABLE agent_chat_run_selections (run_id TEXT PRIMARY KEY NOT NULL); \
                 CREATE TABLE agent_chat_prompt_receipts (request_id TEXT PRIMARY KEY NOT NULL, idempotency_key TEXT NOT NULL, conversation_id TEXT NOT NULL, run_id TEXT NOT NULL, turn_id TEXT NOT NULL, message_id TEXT NOT NULL, disposition TEXT NOT NULL);",
            )
            .unwrap();
        apply(&mut connection).unwrap();
        let identity: String = connection
            .query_row(
                "SELECT identity FROM gent_schema WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(identity, "gent-fresh-schema-v15");
        let column: String = connection
            .query_row(
                "SELECT name FROM pragma_table_info('agent_chat_prompt_receipts') WHERE name = 'tool_source_ids_json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(column, "tool_source_ids_json");
    }
}
