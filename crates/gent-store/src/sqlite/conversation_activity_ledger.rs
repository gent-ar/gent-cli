//! `SQLite` persistence for immutable conversation activity facts.

use gent_core::{activity_scope, validate_conversation_activity_fact};
use gent_ports::{ConversationActivityLedger, LedgerError, MAX_CONVERSATION_ACTIVITY_PAGE_FACTS};
use gent_types::{ConversationActivityFact, ConversationActivityPage};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::SqliteLedger;
use super::conversations::conversation_id_for_run;
use super::queries::{find_run, storage_error};

impl ConversationActivityLedger for SqliteLedger {
    fn append_conversation_activity(
        &self,
        fact: &ConversationActivityFact,
    ) -> Result<(), LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        append(&transaction, fact)?;
        transaction.commit().map_err(storage_error)
    }

    fn read_conversation_activity_page(
        &self,
        conversation_id: &str,
        run_id: &str,
        after_cursor: u64,
        limit: usize,
    ) -> Result<ConversationActivityPage, LedgerError> {
        let connection = self.lock()?;
        page(&connection, conversation_id, run_id, after_cursor, limit)
    }
}

pub(super) fn append(
    connection: &Connection,
    fact: &ConversationActivityFact,
) -> Result<(), LedgerError> {
    validate(connection, fact)?;
    let scope = activity_scope(fact);
    let payload = encode(fact)?;
    let existing = connection
        .query_row(
            "SELECT payload FROM conversation_activity_facts WHERE conversation_id = ?1 AND run_id = ?2 AND cursor = ?3",
            params![scope.conversation_id, scope.run_id, scope.cursor],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?;
    if let Some(existing) = existing {
        return (existing == payload).then_some(()).ok_or_else(|| {
            LedgerError::Invariant("activity cursor conflicts with an immutable fact".into())
        });
    }
    connection.execute(
        "INSERT INTO conversation_activity_facts (conversation_id, run_id, host_epoch, cursor, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![scope.conversation_id, scope.run_id, scope.host_epoch.0, scope.cursor, payload],
    ).map_err(storage_error)?;
    Ok(())
}

fn validate(connection: &Connection, fact: &ConversationActivityFact) -> Result<(), LedgerError> {
    validate_conversation_activity_fact(fact).map_err(LedgerError::Invariant)?;
    let scope = activity_scope(fact);
    if find_run(connection, &scope.run_id)?.is_none() {
        return Err(LedgerError::Invariant("activity run does not exist".into()));
    }
    if conversation_id_for_run(connection, &scope.run_id)?.as_deref()
        != Some(&scope.conversation_id)
    {
        return Err(LedgerError::Invariant(
            "activity run must belong to its conversation".into(),
        ));
    }
    Ok(())
}

fn page(
    connection: &Connection,
    conversation_id: &str,
    run_id: &str,
    after_cursor: u64,
    limit: usize,
) -> Result<ConversationActivityPage, LedgerError> {
    if !(1..=MAX_CONVERSATION_ACTIVITY_PAGE_FACTS).contains(&limit) {
        return Err(LedgerError::Invariant(
            "activity page limit is out of bounds".into(),
        ));
    }
    let mut statement = connection.prepare(
        "SELECT payload FROM conversation_activity_facts WHERE conversation_id = ?1 AND run_id = ?2 AND cursor > ?3 ORDER BY cursor LIMIT ?4",
    ).map_err(storage_error)?;
    let rows = statement
        .query_map(
            params![conversation_id, run_id, after_cursor, limit + 1],
            |row| decode(&row.get::<_, String>(0)?),
        )
        .map_err(storage_error)?;
    let mut facts = rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?;
    let next_after_cursor = (facts.len() > limit)
        .then(|| {
            facts.pop();
            facts.last().map(|fact| activity_scope(fact).cursor)
        })
        .flatten();
    Ok(ConversationActivityPage {
        facts,
        next_after_cursor,
    })
}

fn decode(payload: &str) -> rusqlite::Result<ConversationActivityFact> {
    serde_json::from_str(payload).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn encode(fact: &ConversationActivityFact) -> Result<String, LedgerError> {
    serde_json::to_string(fact).map_err(storage_error)
}
