//! Atomic `SQLite` storage for user-owned, revision-fenced goals.

use gent_ports::{GoalLedger, GoalWrite, LedgerError};
use gent_types::{GoalBinding, GoalRecord, GoalStatus};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{SqliteLedger, queries::storage_error};

impl GoalLedger for SqliteLedger {
    fn find_goal(&self, binding: &GoalBinding) -> Result<Option<GoalRecord>, LedgerError> {
        valid_binding(binding)?;
        find(&*self.lock()?, binding)
    }

    fn create_goal(&self, goal: &GoalRecord) -> Result<GoalWrite, LedgerError> {
        create(self, goal)
    }

    fn replace_goal(
        &self,
        expected: &GoalRecord,
        next: &GoalRecord,
    ) -> Result<GoalWrite, LedgerError> {
        replace(self, expected, next)
    }

    fn conversation_goals(&self, conversation_id: &str) -> Result<Vec<GoalRecord>, LedgerError> {
        list(&*self.lock()?, conversation_id)
    }
}

fn create(ledger: &SqliteLedger, goal: &GoalRecord) -> Result<GoalWrite, LedgerError> {
    valid_create(goal)?;
    let mut connection = ledger.lock()?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_run_binding(&tx, &goal.binding)?;
    match find(&tx, &goal.binding)? {
        Some(current) if current == *goal => Ok(GoalWrite::Current(current)),
        Some(_) => Err(conflict("goal create")),
        None => {
            let owner = tx
                .query_row(
                    "SELECT 1 FROM conversation_goals WHERE goal_id = ?1",
                    [&goal.binding.goal_id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(storage_error)?;
            if owner.is_some() {
                return Err(conflict("goal identity"));
            }
            tx.execute("INSERT INTO conversation_goals (goal_id, conversation_id, run_id, schema_version, revision, status, summary) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![goal.binding.goal_id, goal.binding.conversation_id.0, goal.binding.run_id.0, goal.schema_version, goal.revision, status_name(goal.status), goal.summary]).map_err(storage_error)?;
            tx.commit().map_err(storage_error)?;
            Ok(GoalWrite::Created(goal.clone()))
        }
    }
}

fn replace(
    ledger: &SqliteLedger,
    expected: &GoalRecord,
    next: &GoalRecord,
) -> Result<GoalWrite, LedgerError> {
    valid_replace(expected, next)?;
    let mut connection = ledger.lock()?;
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let current = find(&tx, &expected.binding)?.ok_or_else(|| conflict("goal replacement"))?;
    if current == *expected {
        let changed = tx.execute("UPDATE conversation_goals SET revision = ?1, status = ?2 WHERE goal_id = ?3 AND conversation_id = ?4 AND run_id = ?5 AND revision = ?6", params![next.revision, status_name(next.status), next.binding.goal_id, next.binding.conversation_id.0, next.binding.run_id.0, expected.revision]).map_err(storage_error)?;
        (changed == 1)
            .then_some(())
            .ok_or_else(|| conflict("goal replacement"))?;
        tx.commit().map_err(storage_error)?;
        return Ok(GoalWrite::Updated(next.clone()));
    }
    Ok(GoalWrite::Current(current))
}

fn find(
    connection: &rusqlite::Connection,
    binding: &GoalBinding,
) -> Result<Option<GoalRecord>, LedgerError> {
    connection.query_row("SELECT schema_version, revision, status, summary FROM conversation_goals WHERE goal_id = ?1 AND conversation_id = ?2 AND run_id = ?3", params![binding.goal_id, binding.conversation_id.0, binding.run_id.0], |row| Ok((row.get::<_, u16>(0)?, row.get::<_, u64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))).optional().map_err(storage_error)?.map(|(schema_version, revision, status, summary)| record(binding.clone(), schema_version, revision, &status, summary)).transpose()
}

fn list(
    connection: &rusqlite::Connection,
    conversation: &str,
) -> Result<Vec<GoalRecord>, LedgerError> {
    let mut statement = connection.prepare("SELECT goal_id, run_id, schema_version, revision, status, summary FROM conversation_goals WHERE conversation_id = ?1 ORDER BY creation_order ASC").map_err(storage_error)?;
    let rows = statement
        .query_map([conversation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u16>(2)?,
                row.get::<_, u64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(storage_error)?;
    rows.map(|row| {
        let (goal_id, run_id, schema_version, revision, status, summary) =
            row.map_err(storage_error)?;
        record(
            GoalBinding {
                goal_id,
                conversation_id: gent_types::AgentChatConversationId(conversation.into()),
                run_id: gent_types::AgentChatRunId(run_id),
            },
            schema_version,
            revision,
            &status,
            summary,
        )
    })
    .collect()
}

fn require_run_binding(
    connection: &rusqlite::Connection,
    binding: &GoalBinding,
) -> Result<(), LedgerError> {
    connection
        .query_row(
            "SELECT 1 FROM runs WHERE run_id = ?1 AND conversation_id = ?2",
            params![binding.run_id.0, binding.conversation_id.0],
            |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?
        .is_some()
        .then_some(())
        .ok_or_else(|| conflict("goal ownership binding"))
}

fn valid_create(goal: &GoalRecord) -> Result<(), LedgerError> {
    goal.validate().map_err(|_| conflict("goal metadata"))?;
    (goal.revision == 1 && goal.status == GoalStatus::Active)
        .then_some(())
        .ok_or_else(|| conflict("goal creation state"))
}

fn valid_replace(expected: &GoalRecord, next: &GoalRecord) -> Result<(), LedgerError> {
    expected
        .validate()
        .map_err(|_| conflict("expected goal metadata"))?;
    next.validate()
        .map_err(|_| conflict("replacement goal metadata"))?;
    (expected.binding == next.binding
        && expected.schema_version == next.schema_version
        && expected.summary == next.summary)
        .then_some(())
        .ok_or_else(|| conflict("immutable goal fields"))?;
    (next.revision == expected.revision.checked_add(1).unwrap_or_default())
        .then_some(())
        .ok_or_else(|| conflict("goal revision"))
}

fn record(
    binding: GoalBinding,
    schema_version: u16,
    revision: u64,
    status: &str,
    summary: String,
) -> Result<GoalRecord, LedgerError> {
    let record = GoalRecord {
        schema_version,
        binding,
        revision,
        status: parse_status(status)?,
        summary,
    };
    record
        .validate()
        .map_err(|_| conflict("stored goal metadata"))?;
    Ok(record)
}

const fn status_name(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => "active",
        GoalStatus::Completed => "completed",
        GoalStatus::Abandoned => "abandoned",
        GoalStatus::Failed => "failed",
    }
}

fn parse_status(status: &str) -> Result<GoalStatus, LedgerError> {
    match status {
        "active" => Ok(GoalStatus::Active),
        "completed" => Ok(GoalStatus::Completed),
        "abandoned" => Ok(GoalStatus::Abandoned),
        "failed" => Ok(GoalStatus::Failed),
        _ => Err(conflict("stored goal status")),
    }
}

fn valid_binding(binding: &GoalBinding) -> Result<(), LedgerError> {
    (!binding.goal_id.is_empty()
        && !binding.conversation_id.0.is_empty()
        && !binding.run_id.0.is_empty())
    .then_some(())
    .ok_or_else(|| conflict("goal binding"))
}

fn conflict(subject: &str) -> LedgerError {
    LedgerError::Invariant(format!("goal {subject} conflicts with durable state"))
}
