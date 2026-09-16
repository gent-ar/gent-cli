//! Fresh-only `SQLite` schema creation for the unreleased Gent ledger.

use super::queries::storage_error;
use gent_ports::LedgerError;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
const SCHEMA_ID: &str = "gent-fresh-schema-v24";
const V23_SCHEMA_ID: &str = "gent-fresh-schema-v23";
const V22_SCHEMA_ID: &str = "gent-fresh-schema-v22";
const V21_SCHEMA_ID: &str = "gent-fresh-schema-v21";
const V20_SCHEMA_ID: &str = "gent-fresh-schema-v20";
const V19_SCHEMA_ID: &str = "gent-fresh-schema-v19";
const V18_SCHEMA_ID: &str = "gent-fresh-schema-v18";
const V17_SCHEMA_ID: &str = "gent-fresh-schema-v17";
const V16_SCHEMA_ID: &str = "gent-fresh-schema-v16";
const V15_SCHEMA_ID: &str = "gent-fresh-schema-v15";
const V10_SCHEMA_ID: &str = "gent-fresh-schema-v10";
const SCHEMA: &str = concat!(
    include_str!("fresh_schema.sql"),
    include_str!("fresh_schema_agent_chat.sql"),
    include_str!("fresh_schema_projection.sql"),
    include_str!("fresh_schema_conversation_recency.sql")
);
const V10_TO_V15: &str = include_str!("fresh_schema_v10_to_v15.sql");
const V15_TO_V16: &str = include_str!("fresh_schema_v15_to_v16.sql");
const V16_TO_V17: &str = include_str!("fresh_schema_v16_to_v17.sql");
const V17_TO_V18: &str = include_str!("fresh_schema_v17_to_v18.sql");
const V18_TO_V19: &str = include_str!("fresh_schema_v18_to_v19.sql");
const V19_TO_V20: &str = include_str!("fresh_schema_v19_to_v20.sql");
const V20_TO_V21: &str = include_str!("fresh_schema_v20_to_v21.sql");
const V21_TO_V22: &str = include_str!("fresh_schema_v21_to_v22.sql");
const V22_TO_V23: &str = concat!(
    include_str!("fresh_schema_v22_to_v23.sql"),
    include_str!("fresh_schema_conversation_recency.sql")
);
const V23_TO_V24: &str = include_str!("fresh_schema_v23_to_v24.sql");

/// Opens an empty database with Gent's complete current schema.
pub(super) fn apply(connection: &mut Connection) -> Result<(), LedgerError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    if let Some(identity) = schema_identity(&transaction)? {
        if identity == SCHEMA_ID {
            return Ok(());
        }
        upgrade(&transaction, &identity)?;
        transaction
            .execute(
                "UPDATE gent_schema SET identity = ?1 WHERE singleton = 1",
                [SCHEMA_ID],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        return Ok(());
    }
    if has_user_tables(&transaction)? {
        return Err(LedgerError::Invariant(
            "existing Gent ledger requires reset; this build accepts only an empty database".into(),
        ));
    }
    transaction.execute_batch(SCHEMA).map_err(storage_error)?;
    transaction.commit().map_err(storage_error)
}

fn upgrade_through_v18(
    transaction: &rusqlite::Transaction<'_>,
    identity: &str,
) -> Result<bool, LedgerError> {
    let requires_v15_to_v16 = if identity == V10_SCHEMA_ID {
        transaction
            .execute_batch(V10_TO_V15)
            .map_err(storage_error)?;
        if !has_column(
            transaction,
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
        true
    } else {
        identity == V15_SCHEMA_ID
    };
    if requires_v15_to_v16 {
        if has_table(transaction, "policies")? {
            transaction
                .execute(
                    "UPDATE policies SET mode = 'askEveryTime' WHERE mode IN ('default', 'plan')",
                    [],
                )
                .map_err(storage_error)?;
        }
        transaction
            .execute_batch(V15_TO_V16)
            .map_err(storage_error)?;
    }
    let requires_v17_to_v18 = if requires_v15_to_v16 || identity == V16_SCHEMA_ID {
        transaction
            .execute_batch(V16_TO_V17)
            .map_err(storage_error)?;
        true
    } else {
        identity == V17_SCHEMA_ID
    };
    Ok(requires_v17_to_v18)
}

fn upgrade(transaction: &rusqlite::Transaction<'_>, identity: &str) -> Result<(), LedgerError> {
    let requires_v17_to_v18 = upgrade_through_v18(transaction, identity)?;
    let requires_v18_to_v19 = if requires_v17_to_v18 {
        transaction
            .execute_batch(V17_TO_V18)
            .map_err(storage_error)?;
        true
    } else {
        identity == V18_SCHEMA_ID
    };
    let requires_v19_to_v20 = if requires_v18_to_v19 {
        transaction
            .execute_batch(V18_TO_V19)
            .map_err(storage_error)?;
        true
    } else {
        identity == V19_SCHEMA_ID
    };
    let requires_v20_to_v21 = if requires_v19_to_v20 {
        transaction
            .execute_batch(V19_TO_V20)
            .map_err(storage_error)?;
        true
    } else {
        identity == V20_SCHEMA_ID
    };
    let requires_v21_to_v22 = if requires_v20_to_v21 {
        transaction
            .execute_batch(V20_TO_V21)
            .map_err(storage_error)?;
        true
    } else {
        identity == V21_SCHEMA_ID
    };
    let requires_v22_to_v23 = if requires_v21_to_v22 {
        if has_table(transaction, "agent_chat_transcript_events")? {
            transaction
                .execute_batch(V21_TO_V22)
                .map_err(storage_error)?;
        }
        true
    } else {
        identity == V22_SCHEMA_ID
    };
    let requires_v23_to_v24 = if requires_v22_to_v23 {
        if has_tables(
            transaction,
            &[
                "agent_chat_conversations",
                "agent_chat_projection_events",
                "agent_chat_transcript_events",
                "turns",
            ],
        )? {
            transaction
                .execute_batch(V22_TO_V23)
                .map_err(storage_error)?;
        }
        true
    } else {
        identity == V23_SCHEMA_ID
    };
    if !requires_v23_to_v24 {
        return Err(LedgerError::Invariant(
            "unknown Gent fresh schema identity".into(),
        ));
    }
    transaction
        .execute_batch(V23_TO_V24)
        .map_err(storage_error)?;
    Ok(())
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

fn has_table(connection: &Connection, table: &str) -> Result<bool, LedgerError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            [table],
            |_| Ok(()),
        )
        .optional()
        .map(|row| row.is_some())
        .map_err(storage_error)
}

fn has_tables(connection: &Connection, tables: &[&str]) -> Result<bool, LedgerError> {
    for table in tables {
        if !has_table(connection, table)? {
            return Ok(false);
        }
    }
    Ok(true)
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
#[path = "fresh_schema_tests.rs"]
mod tests;
