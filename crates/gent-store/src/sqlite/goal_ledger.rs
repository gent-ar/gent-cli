//! Atomic `SQLite` storage for user-owned, revision-fenced goals.

use gent_ports::{GoalLedger, GoalWrite, IngressMode, LedgerError};
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, Event, GoalRecord, GoalStatus,
    GoalTurnObservation, HostEpoch, ReceiptId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use super::epoch::require_epoch;
use super::queries::{append_event, host_ingress, storage_error};
use super::{SqliteLedger, conversation_activity_ledger};

#[path = "goal_turns.rs"]
mod turns;

const COLUMNS: &str = "SELECT record_json FROM conversation_goals";

impl GoalLedger for SqliteLedger {
    fn current_goal(&self, conversation_id: &str) -> Result<Option<GoalRecord>, LedgerError> {
        current(&*self.lock()?, conversation_id)
    }

    fn find_goal(&self, goal_id: &str) -> Result<Option<GoalRecord>, LedgerError> {
        self.lock()?
            .query_row(&format!("{COLUMNS} WHERE goal_id = ?1"), [goal_id], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(storage_error)?
            .map(|json| decode(&json))
            .transpose()
    }

    fn active_goals(&self) -> Result<Vec<GoalRecord>, LedgerError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "{COLUMNS} WHERE status = 'active' ORDER BY creation_order ASC LIMIT 256"
            ))
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(storage_error)?;
        rows.map(|row| decode(&row.map_err(storage_error)?))
            .collect()
    }

    fn create_goal(
        &self,
        expected_current: Option<&GoalRecord>,
        replaced: Option<&GoalRecord>,
        goal: &GoalRecord,
        host_epoch: HostEpoch,
    ) -> Result<GoalWrite, LedgerError> {
        valid(goal)?;
        let mut connection = self.lock()?;
        let tx = open(&mut connection, host_epoch)?;
        let current = current(&tx, &goal.binding.conversation_id.0)?;
        if current.as_ref() != expected_current {
            return Ok(GoalWrite::Current(current));
        }
        if let (Some(expected), Some(next)) = (expected_current, replaced) {
            update(&tx, expected, next, host_epoch)?;
        }
        tx.execute(
            "INSERT INTO conversation_goals (goal_id, conversation_id, revision, status, record_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![goal.binding.goal_id, goal.binding.conversation_id.0, goal.revision, status_name(goal.status), encode(goal)?],
        )
        .map_err(storage_error)?;
        publish(&tx, goal, host_epoch)?;
        tx.commit().map_err(storage_error)?;
        Ok(GoalWrite::Updated(goal.clone()))
    }

    fn replace_goal(
        &self,
        expected: &GoalRecord,
        next: &GoalRecord,
        host_epoch: HostEpoch,
    ) -> Result<GoalWrite, LedgerError> {
        valid(next)?;
        let mut connection = self.lock()?;
        let tx = open(&mut connection, host_epoch)?;
        let stored = tx
            .query_row(
                &format!("{COLUMNS} WHERE goal_id = ?1"),
                [&expected.binding.goal_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?
            .map(|json| decode(&json))
            .transpose()?;
        if stored.as_ref() != Some(expected) {
            return Ok(GoalWrite::Current(stored));
        }
        update(&tx, expected, next, host_epoch)?;
        tx.commit().map_err(storage_error)?;
        Ok(GoalWrite::Updated(next.clone()))
    }

    fn goal_turns(
        &self,
        conversation_id: &str,
        after_ordinal: u64,
    ) -> Result<Vec<GoalTurnObservation>, LedgerError> {
        turns::observe(&*self.lock()?, conversation_id, after_ordinal)
    }

    fn latest_turn_ordinal(&self, conversation_id: &str) -> Result<u64, LedgerError> {
        self.lock()?
            .query_row(
                "SELECT COALESCE(MAX(ordinal), 0) FROM conversation_message_ordinals WHERE conversation_id = ?1",
                [conversation_id],
                |row| row.get(0),
            )
            .map_err(storage_error)
    }
}

fn open(
    connection: &mut Connection,
    host_epoch: HostEpoch,
) -> Result<Transaction<'_>, LedgerError> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&tx)?;
    require_epoch(host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    Ok(tx)
}

fn update(
    tx: &Transaction<'_>,
    expected: &GoalRecord,
    next: &GoalRecord,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    if next.binding != expected.binding
        || next.revision != expected.revision.saturating_add(1)
        || next.created_at != expected.created_at
        || next.objective != expected.objective
    {
        return Err(conflict("immutable goal fields"));
    }
    let changed = tx
        .execute(
            "UPDATE conversation_goals SET revision = ?1, status = ?2, record_json = ?3 WHERE goal_id = ?4 AND revision = ?5",
            params![next.revision, status_name(next.status), encode(next)?, next.binding.goal_id, expected.revision],
        )
        .map_err(storage_error)?;
    if changed != 1 {
        return Err(conflict("goal revision"));
    }
    publish(tx, next, host_epoch)
}

fn publish(
    tx: &Transaction<'_>,
    goal: &GoalRecord,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    let conversation_id = &goal.binding.conversation_id.0;
    let run_id: String = tx
        .query_row(
            "SELECT run_id FROM runs WHERE conversation_id = ?1 ORDER BY rowid DESC LIMIT 1",
            [conversation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| conflict("goal conversation"))?;
    let turn_id = format!("goal:{}", goal.binding.goal_id);
    let identity = format!("goal:{}:{}", goal.binding.goal_id, goal.revision);
    let fact = ConversationActivityFact::GoalUpdated {
        scope: ConversationActivityScope {
            conversation_id: conversation_id.clone(),
            run_id: run_id.clone(),
            turn_id: turn_id.clone(),
            host_epoch,
            cursor: 0,
        },
        goal: goal.clone(),
    };
    let event = append_event(
        tx,
        &Event {
            cursor: 0,
            event_id: identity.clone(),
            receipt_id: ReceiptId(identity),
            host_epoch,
            kind: "agentChatGoalActivity".into(),
            payload: serde_json::json!({
                "conversationId": conversation_id,
                "runId": run_id,
                "turnId": turn_id,
                "activity": fact,
            }),
        },
    )?;
    let fact = gent_core::with_activity_cursor(fact, event.cursor);
    conversation_activity_ledger::append(tx, &fact)?;
    tx.execute(
        "INSERT INTO agent_chat_projection_events (conversation_id, source_event_id, kind, payload) VALUES (?1, ?2, 'activity', ?3)",
        params![
            conversation_id,
            format!("activity:{}", event.event_id),
            serde_json::json!({
                "conversationId": conversation_id,
                "runId": run_id,
                "turnId": turn_id,
                "activity": fact,
            })
            .to_string(),
        ],
    )
    .map_err(storage_error)?;
    Ok(())
}

fn current(
    connection: &Connection,
    conversation_id: &str,
) -> Result<Option<GoalRecord>, LedgerError> {
    connection
        .query_row(
            &format!("{COLUMNS} WHERE conversation_id = ?1 ORDER BY creation_order DESC LIMIT 1"),
            [conversation_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?
        .map(|json| decode(&json))
        .transpose()
}

pub(super) fn decode_origin(
    value: Option<String>,
) -> rusqlite::Result<Option<gent_types::AgentChatPromptOrigin>> {
    value
        .map(|json| {
            serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

fn valid(goal: &GoalRecord) -> Result<(), LedgerError> {
    goal.validate().map_err(|_| conflict("goal metadata"))
}

fn encode(goal: &GoalRecord) -> Result<String, LedgerError> {
    serde_json::to_string(goal).map_err(storage_error)
}

fn decode(json: &str) -> Result<GoalRecord, LedgerError> {
    let goal: GoalRecord = serde_json::from_str(json).map_err(|_| conflict("stored goal"))?;
    valid(&goal)?;
    Ok(goal)
}

const fn status_name(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => "active",
        GoalStatus::Paused => "paused",
        GoalStatus::Blocked => "blocked",
        GoalStatus::UsageLimited => "usageLimited",
        GoalStatus::BudgetLimited => "budgetLimited",
        GoalStatus::Complete => "complete",
        GoalStatus::Cleared => "cleared",
    }
}

fn conflict(subject: &str) -> LedgerError {
    LedgerError::Invariant(format!("goal {subject} conflicts with durable state"))
}
