use std::collections::BTreeSet;

use gent_ports::LedgerError;
use gent_types::{
    AgentChatProjectionEvent, ConversationTurnCursor, ConversationTurnCursorPage,
    MAX_PROJECTION_WINDOW_CARRY, MAX_TURN_CURSOR_PAGE, ProjectionWindowCarry, TurnRecord,
};
use rusqlite::{Connection, OptionalExtension, params};

use super::agent_chat_projection_ledger::{COLUMNS, event};
use super::{SqliteLedger, queries::storage_error};

pub(super) const CARRIED_TURNS: [&str; 3] = [
    "SELECT turn_id FROM agent_chat_projection_turns WHERE conversation_id = ?1 AND last_projection_cursor >= ?2 AND first_projection_cursor < ?2",
    "SELECT turn_id FROM turns WHERE conversation_id = ?1 AND phase NOT IN ('completed', 'interrupted', 'cancelled', 'failed') AND ?2 > 0",
    "SELECT json_extract(binding_json, '$.turnId') FROM pending_provider_permissions WHERE conversation_id = ?1 AND ?2 > 0",
];
pub(super) const CONVERSATION_RUNS: &str =
    "SELECT run_id FROM runs WHERE conversation_id = ?1 AND ?2 > 0";
pub(super) const TURN_CURSOR_PAGE: &str = "SELECT turn_id, run_id, first_projection_cursor, first_transcript_cursor, first_activity_cursor FROM agent_chat_projection_turns WHERE conversation_id = ?1 AND first_projection_cursor < ?2 ORDER BY first_projection_cursor DESC LIMIT ?3";
pub(super) const TURN_FACTS: &str = "WHERE conversation_id = ?1 AND json_extract(payload, '$.turnId') = ?2 AND cursor < ?3 ORDER BY cursor";
pub(super) const LATEST_ACTIVITY: &str = "WHERE conversation_id = ?1 AND kind = 'activity' AND json_extract(payload, '$.activity.type') = ?2 AND cursor < ?3 ORDER BY cursor DESC LIMIT ?4";
pub(super) const LATEST_SUMMARY: &str =
    "WHERE conversation_id = ?1 AND kind = 'summary' AND cursor < ?2 ORDER BY cursor DESC LIMIT 1";
const LATEST_TYPES: [(&str, u32); 5] = [
    ("goalUpdated", 1),
    ("planUpdated", 1),
    ("conversationCreated", 256),
    ("createdByConversation", 1),
    ("sideQuestionSettled", 32),
];

impl SqliteLedger {
    pub fn conversation_turn_cursors(
        &self,
        conversation_id: &str,
        before_cursor: Option<u64>,
        limit: u16,
    ) -> Result<ConversationTurnCursorPage, LedgerError> {
        if limit == 0 || limit > MAX_TURN_CURSOR_PAGE {
            return Err(LedgerError::Invariant(
                "turn cursor page limit is out of range".into(),
            ));
        }
        let connection = self.lock()?;
        let mut statement = connection
            .prepare_cached(TURN_CURSOR_PAGE)
            .map_err(storage_error)?;
        let before =
            before_cursor.map_or(i64::MAX, |cursor| i64::try_from(cursor).unwrap_or(i64::MAX));
        let turns = statement
            .query_map(params![conversation_id, before, limit], |row| {
                Ok(ConversationTurnCursor {
                    turn_id: row.get(0)?,
                    run_id: row.get(1)?,
                    first_projection_cursor: row.get(2)?,
                    first_transcript_cursor: row.get(3)?,
                    first_activity_cursor: row.get(4)?,
                })
            })
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        Ok(ConversationTurnCursorPage {
            next_before_cursor: (turns.len() == usize::from(limit))
                .then(|| turns.last().map(|turn| turn.first_projection_cursor))
                .flatten(),
            turns,
        })
    }

    pub fn projection_window_carry(
        &self,
        conversation_id: &str,
        from_cursor: u64,
    ) -> Result<ProjectionWindowCarry, LedgerError> {
        let connection = self.lock()?;
        let mut turns = BTreeSet::new();
        for query in CARRIED_TURNS {
            let mut statement = connection.prepare_cached(query).map_err(storage_error)?;
            let rows = statement
                .query_map(params![conversation_id, from_cursor], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .map_err(storage_error)?;
            for row in rows {
                turns.extend(row.map_err(storage_error)?);
            }
        }
        let mut statement = connection
            .prepare_cached(CONVERSATION_RUNS)
            .map_err(storage_error)?;
        let runs = statement
            .query_map(params![conversation_id, from_cursor], |row| {
                row.get::<_, String>(0)
            })
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;
        for run in runs {
            turns.extend(super::run_live_status::open_work_turns(&connection, &run)?);
        }
        let mut events = Vec::new();
        for turn in &turns {
            events.extend(read(
                &connection,
                &format!("{COLUMNS} {TURN_FACTS}"),
                params![conversation_id, turn, from_cursor],
            )?);
        }
        for (activity, limit) in LATEST_TYPES {
            events.extend(read(
                &connection,
                &format!("{COLUMNS} {LATEST_ACTIVITY}"),
                params![conversation_id, activity, from_cursor, limit],
            )?);
        }
        events.extend(read(
            &connection,
            &format!("{COLUMNS} {LATEST_SUMMARY}"),
            params![conversation_id, from_cursor],
        )?);
        events.sort_by_key(|event| event.cursor);
        events.dedup_by_key(|event| event.cursor);
        if events.len() > MAX_PROJECTION_WINDOW_CARRY {
            return Err(LedgerError::Invariant(
                "the projection window carries more facts than one read may return".into(),
            ));
        }
        for event in &mut events {
            super::historical_tool_output::project(&connection, event)?;
        }
        super::transcript_attachments::attach_to_projection(&connection, &mut events)?;
        Ok(ProjectionWindowCarry {
            last_turn: last_turn(&connection, conversation_id, from_cursor)?,
            events,
        })
    }
}

fn read(
    connection: &Connection,
    query: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<AgentChatProjectionEvent>, LedgerError> {
    let mut statement = connection.prepare_cached(query).map_err(storage_error)?;
    statement
        .query_map(params, event)
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)
}

fn last_turn(
    connection: &Connection,
    conversation_id: &str,
    from_cursor: u64,
) -> Result<Option<TurnRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT t.turn_id, t.conversation_id, t.run_id, t.sequence, t.phase FROM agent_chat_projection_turns b JOIN turns t ON t.turn_id = b.turn_id WHERE b.conversation_id = ?1 AND b.first_projection_cursor < ?2 ORDER BY b.first_projection_cursor DESC LIMIT 1",
            params![conversation_id, from_cursor],
            |row| {
                Ok(TurnRecord {
                    turn_id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    run_id: row.get(2)?,
                    sequence: row.get(3)?,
                    phase: super::conversations::decode_phase(&row.get::<_, String>(4)?)?,
                })
            },
        )
        .optional()
        .map_err(storage_error)
}

#[cfg(test)]
#[path = "projection_window_tests.rs"]
mod tests;
