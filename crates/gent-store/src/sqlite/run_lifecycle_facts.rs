use gent_ports::{LedgerError, RunLifecycleFactLedger};
use gent_types::{RunLifecycleFact, RunLifecycleFactPage, RunLiveStatus};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{SqliteLedger, queries::storage_error};

const MAX_PAGE_SIZE: usize = 128;

impl RunLifecycleFactLedger for SqliteLedger {
    fn append_run_lifecycle_fact(&self, fact: &RunLifecycleFact) -> Result<(), LedgerError> {
        let connection = self.lock()?;
        append(&connection, fact)
    }

    fn read_run_lifecycle_fact_page(
        &self,
        run_id: &str,
        after_cursor: u64,
        limit: usize,
    ) -> Result<RunLifecycleFactPage, LedgerError> {
        let connection = self.lock()?;
        read(&connection, run_id, after_cursor, limit)
    }

    fn settle_run_permissions(&self, run_id: &str) -> Result<(), LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        super::pending_permission_ledger::settle_for_run(&transaction, run_id)?;
        transaction.commit().map_err(storage_error)
    }

    fn run_live_status(&self, run_id: &str) -> Result<Option<RunLiveStatus>, LedgerError> {
        let connection = self.lock()?;
        super::run_live_status::read(&connection, run_id)
    }

    fn end_background_tools(
        &self,
        conversation_id: &str,
        phase: gent_types::ToolPhase,
    ) -> Result<(), LedgerError> {
        end_background(
            self,
            &super::background_tools::Scope::Conversation(conversation_id),
            &phase,
        )
    }

    fn end_run_background_tools(
        &self,
        run_id: &str,
        phase: gent_types::ToolPhase,
    ) -> Result<(), LedgerError> {
        end_background(self, &super::background_tools::Scope::Run(run_id), &phase)
    }

    fn run_active_turn(&self, run_id: &str) -> Result<Option<String>, LedgerError> {
        let connection = self.lock()?;
        super::run_live_status::latest_turn(&connection, run_id).map(|turn| {
            turn.filter(|(_, phase)| !phase.is_terminal())
                .map(|(turn_id, _)| turn_id)
        })
    }
}

fn end_background(
    ledger: &SqliteLedger,
    scope: &super::background_tools::Scope<'_>,
    phase: &gent_types::ToolPhase,
) -> Result<(), LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    super::background_tools::end(&transaction, scope, phase, None)?;
    transaction.commit().map_err(storage_error)
}

pub(super) fn append(connection: &Connection, fact: &RunLifecycleFact) -> Result<(), LedgerError> {
    validate(fact)?;
    validate_source(connection, fact)?;
    let payload = serde_json::to_string(fact).map_err(storage_error)?;
    let existing = connection
        .query_row(
            "SELECT payload FROM run_lifecycle_facts WHERE run_id = ?1 AND cursor = ?2",
            params![fact.run_id, fact.cursor],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?;
    if let Some(existing) = existing {
        return (existing == payload).then_some(()).ok_or_else(|| {
            LedgerError::Invariant("lifecycle fact cursor conflicts with durable fact".into())
        });
    }
    connection
        .execute(
            "INSERT INTO run_lifecycle_facts (run_id, cursor, event_id, host_epoch, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![fact.run_id, fact.cursor, fact.event_id, fact.host_epoch.0, payload],
        )
        .map(|_| ())
        .map_err(storage_error)
}

pub(super) fn append_in_transaction(
    transaction: &Transaction<'_>,
    fact: &RunLifecycleFact,
) -> Result<(), LedgerError> {
    append(transaction, fact)
}

pub(super) fn read(
    connection: &Connection,
    run_id: &str,
    after_cursor: u64,
    limit: usize,
) -> Result<RunLifecycleFactPage, LedgerError> {
    let limit = page_limit(limit)?;
    let mut statement = connection
        .prepare(
            "SELECT payload FROM run_lifecycle_facts WHERE run_id = ?1 AND cursor > ?2 ORDER BY cursor ASC LIMIT ?3",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map(params![run_id, after_cursor, limit + 1], |row| {
            row.get::<_, String>(0)
        })
        .map_err(storage_error)?;
    let facts = rows
        .map(|row| {
            serde_json::from_str(&row.map_err(storage_error)?)
                .map_err(|error| LedgerError::Storage(error.to_string()))
        })
        .collect::<Result<Vec<RunLifecycleFact>, LedgerError>>()?;
    let has_more = facts.len() > limit;
    let facts = facts.into_iter().take(limit).collect::<Vec<_>>();
    Ok(RunLifecycleFactPage {
        next_after_cursor: has_more.then(|| facts.last().map_or(after_cursor, |fact| fact.cursor)),
        facts,
    })
}

fn validate(fact: &RunLifecycleFact) -> Result<(), LedgerError> {
    if fact.run_id.trim().is_empty() || fact.event_id.trim().is_empty() || fact.cursor == 0 {
        return Err(LedgerError::Invariant(
            "lifecycle fact identity is invalid".into(),
        ));
    }
    Ok(())
}

fn validate_source(connection: &Connection, fact: &RunLifecycleFact) -> Result<(), LedgerError> {
    let source = connection
        .query_row(
            "SELECT host_epoch, kind, payload FROM events WHERE cursor = ?1 AND event_id = ?2",
            params![fact.cursor, fact.event_id],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    let Some((epoch, kind, payload)) = source else {
        return Err(LedgerError::Invariant(
            "lifecycle fact source event is absent".into(),
        ));
    };
    let value = serde_json::from_str::<serde_json::Value>(&payload)
        .map_err(|error| LedgerError::Storage(error.to_string()))?;
    let source_run_id = value.get("runId").and_then(|value| value.as_str());
    let source_lifecycle = match kind.as_str() {
        "normalizedSessionLifecycle" => value.get("lifecycle").cloned(),
        "providerLifecycle" => value
            .get("event")
            .map(|event| serde_json::json!({ "type": "event", "event": event }))
            .or_else(|| {
                value
                    .get("signal")
                    .map(|signal| serde_json::json!({ "type": "signal", "signal": signal }))
            }),
        _ => None,
    }
    .map(serde_json::from_value)
    .transpose()
    .map_err(|error| LedgerError::Storage(error.to_string()))?;
    let lifecycle_matches = source_lifecycle.as_ref() == Some(&fact.lifecycle);
    if epoch != fact.host_epoch.0 || source_run_id != Some(&fact.run_id) || !lifecycle_matches {
        return Err(LedgerError::Invariant(
            "lifecycle fact does not match its durable source".into(),
        ));
    }
    Ok(())
}

fn page_limit(limit: usize) -> Result<usize, LedgerError> {
    (1..=MAX_PAGE_SIZE)
        .contains(&limit)
        .then_some(limit)
        .ok_or_else(|| LedgerError::Invariant("lifecycle fact page limit is invalid".into()))
}
