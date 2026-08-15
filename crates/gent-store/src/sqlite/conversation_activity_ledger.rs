//! `SQLite` persistence for complete conversation activity reducer state.

use gent_ports::{
    ConversationActivityLedger, LedgerError, MAX_CONVERSATION_ACTIVITY_RESUME_RECORDS,
};
use gent_types::ConversationActivityRecord;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::SqliteLedger;
use super::conversations::conversation_id_for_run;
use super::queries::{find_run, storage_error};

impl ConversationActivityLedger for SqliteLedger {
    fn save_conversation_activity(
        &self,
        record: &ConversationActivityRecord,
    ) -> Result<(), LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        validate(&transaction, record)?;
        if let Some(existing) = find_at_cursor(&transaction, record)? {
            if existing == *record {
                return Ok(());
            }
            return Err(LedgerError::Invariant(
                "activity cursor conflicts with existing state".into(),
            ));
        }
        match find(
            &transaction,
            &record.activity.conversation_id,
            &record.activity.run_id,
        )? {
            Some(current) => save_after_current(&transaction, &current, record)?,
            None => insert(&transaction, record)?,
        }
        transaction.commit().map_err(storage_error)
    }

    fn find_conversation_activity(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Option<ConversationActivityRecord>, LedgerError> {
        let connection = self.lock()?;
        find(&connection, conversation_id, run_id)
    }

    fn resume_conversation_activity(
        &self,
        conversation_id: &str,
        run_id: &str,
        after_cursor: u64,
    ) -> Result<Vec<ConversationActivityRecord>, LedgerError> {
        let connection = self.lock()?;
        resume(&connection, conversation_id, run_id, after_cursor)
    }
}

fn validate(
    connection: &Connection,
    record: &ConversationActivityRecord,
) -> Result<(), LedgerError> {
    let activity = &record.activity;
    if activity.activity_sequence == 0 || activity.revision != activity.activity_sequence {
        return Err(LedgerError::Invariant(
            "activity revision and sequence must be equal and nonzero".into(),
        ));
    }
    if find_run(connection, &activity.run_id)?.is_none() {
        return Err(LedgerError::Invariant("activity run does not exist".into()));
    }
    if conversation_id_for_run(connection, &activity.run_id)?.as_deref()
        != Some(&activity.conversation_id)
    {
        return Err(LedgerError::Invariant(
            "activity run must belong to its conversation".into(),
        ));
    }
    Ok(())
}

fn save_after_current(
    connection: &Connection,
    current: &ConversationActivityRecord,
    next: &ConversationActivityRecord,
) -> Result<(), LedgerError> {
    let previous = &current.activity;
    let activity = &next.activity;
    if activity.host_epoch != previous.host_epoch {
        return Err(LedgerError::Invariant(
            "activity host epoch cannot change in place".into(),
        ));
    }
    if activity.cursor <= previous.cursor
        || activity.revision <= previous.revision
        || activity.activity_sequence <= previous.activity_sequence
    {
        return Err(LedgerError::Invariant("activity ordering regressed".into()));
    }
    insert(connection, next)
}

fn insert(connection: &Connection, record: &ConversationActivityRecord) -> Result<(), LedgerError> {
    connection
        .execute(
            "INSERT INTO conversation_activity_projection_journal (conversation_id, run_id, host_epoch, cursor, revision, activity_sequence, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![record.activity.conversation_id, record.activity.run_id, record.activity.host_epoch.0, record.activity.cursor, record.activity.revision, record.activity.activity_sequence, encode(record)?],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn find(
    connection: &Connection,
    conversation_id: &str,
    run_id: &str,
) -> Result<Option<ConversationActivityRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT payload FROM conversation_activity_projection_journal WHERE conversation_id = ?1 AND run_id = ?2 ORDER BY cursor DESC LIMIT 1",
            params![conversation_id, run_id],
            |row| decode_payload(&row.get::<_, String>(0)?),
        )
        .optional()
        .map_err(storage_error)
}

fn find_at_cursor(
    connection: &Connection,
    record: &ConversationActivityRecord,
) -> Result<Option<ConversationActivityRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT payload FROM conversation_activity_projection_journal WHERE conversation_id = ?1 AND run_id = ?2 AND cursor = ?3",
            params![record.activity.conversation_id, record.activity.run_id, record.activity.cursor],
            |row| decode_payload(&row.get::<_, String>(0)?),
        )
        .optional()
        .map_err(storage_error)
}

fn resume(
    connection: &Connection,
    conversation_id: &str,
    run_id: &str,
    after_cursor: u64,
) -> Result<Vec<ConversationActivityRecord>, LedgerError> {
    let mut statement = connection
        .prepare(
            "SELECT payload FROM conversation_activity_projection_journal WHERE conversation_id = ?1 AND run_id = ?2 AND cursor > ?3 ORDER BY cursor LIMIT ?4",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map(
            params![
                conversation_id,
                run_id,
                after_cursor,
                MAX_CONVERSATION_ACTIVITY_RESUME_RECORDS
            ],
            |row| decode_payload(&row.get::<_, String>(0)?),
        )
        .map_err(storage_error)?;
    rows.collect::<Result<_, _>>().map_err(storage_error)
}

fn decode_payload(payload: &str) -> rusqlite::Result<ConversationActivityRecord> {
    serde_json::from_str(payload).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn encode(record: &ConversationActivityRecord) -> Result<String, LedgerError> {
    serde_json::to_string(record).map_err(storage_error)
}
