use std::collections::HashSet;

use gent_ports::{AgentChatProjectionLedger, LedgerError};
use gent_types::{
    AgentChatConversationId, AgentChatProjectionEvent, AgentChatProjectionPage,
    AgentChatProjectionTail,
};
use rusqlite::{Connection, Row, params};

use super::{SqliteLedger, queries::storage_error};

const MAX_PAGE_LIMIT: u16 = 100;
const COLUMNS: &str =
    "SELECT cursor, source_event_id, kind, payload FROM agent_chat_projection_events";

impl AgentChatProjectionLedger for SqliteLedger {
    fn agent_chat_projection_page(
        &self,
        conversation_id: &AgentChatConversationId,
        after_cursor: u64,
        limit: u16,
    ) -> Result<AgentChatProjectionPage, LedgerError> {
        validate_limit(limit)?;
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "{COLUMNS} WHERE conversation_id = ?1 AND cursor > ?2 ORDER BY cursor ASC LIMIT ?3"
            ))
            .map_err(storage_error)?;
        let rows = statement
            .query_map(
                params![conversation_id.0, after_cursor, i64::from(limit) + 1],
                event,
            )
            .map_err(storage_error)?;
        let mut events = rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?;
        let has_next = events.len() > usize::from(limit);
        events.truncate(usize::from(limit));
        super::transcript_attachments::attach_to_projection(&connection, &mut events)?;
        Ok(AgentChatProjectionPage {
            conversation_id: conversation_id.0.clone(),
            next_after_cursor: has_next
                .then(|| events.last().map_or(after_cursor, |event| event.cursor)),
            events,
        })
    }

    fn agent_chat_projection_tail(
        &self,
        conversation_id: &AgentChatConversationId,
        transcript_limit: u16,
        activity_limit: u16,
    ) -> Result<AgentChatProjectionTail, LedgerError> {
        validate_limit(transcript_limit)?;
        validate_limit(activity_limit)?;
        let connection = self.lock()?;
        let cursor = connection
            .query_row(
                "SELECT COALESCE(MAX(cursor), 0) FROM agent_chat_projection_events WHERE conversation_id = ?1",
                [&conversation_id.0],
                |row| row.get::<_, u64>(0),
            )
            .map_err(storage_error)?;
        let (transcript, transcript_truncated) = latest(
            &connection,
            conversation_id,
            "transcript",
            cursor,
            transcript_limit,
        )?;
        let (activity, activity_truncated) = latest(
            &connection,
            conversation_id,
            "activity",
            cursor,
            activity_limit,
        )?;
        Ok(AgentChatProjectionTail {
            conversation_id: conversation_id.0.clone(),
            cursor,
            transcript,
            activity,
            transcript_truncated,
            activity_truncated,
        })
    }
}

fn latest(
    connection: &Connection,
    conversation_id: &AgentChatConversationId,
    kind: &str,
    through_cursor: u64,
    limit: u16,
) -> Result<(Vec<AgentChatProjectionEvent>, bool), LedgerError> {
    let mut statement = connection
        .prepare(&format!("{COLUMNS} WHERE conversation_id = ?1 AND kind = ?2 AND cursor <= ?3 ORDER BY cursor DESC"))
        .map_err(storage_error)?;
    let mut rows = statement
        .query_map(params![conversation_id.0, kind, through_cursor], event)
        .map_err(storage_error)?;
    let mut settled = HashSet::new();
    let mut window = Vec::new();
    for row in rows.by_ref() {
        let event = row.map_err(storage_error)?;
        if kind == "transcript" && superseded(&event, &mut settled) {
            continue;
        }
        if window.len() == usize::from(limit) {
            return attached(connection, window, true);
        }
        window.push(event);
    }
    attached(connection, window, false)
}

fn attached(
    connection: &Connection,
    mut window: Vec<AgentChatProjectionEvent>,
    truncated: bool,
) -> Result<(Vec<AgentChatProjectionEvent>, bool), LedgerError> {
    window.reverse();
    super::transcript_attachments::attach_to_projection(connection, &mut window)?;
    Ok((window, truncated))
}

fn superseded(event: &AgentChatProjectionEvent, settled: &mut HashSet<(String, String)>) -> bool {
    let text = |field: &str| {
        event
            .payload
            .get(field)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let identity = (text("turnId"), text("kind"));
    let partial = event
        .payload
        .get("isPartial")
        .is_some_and(|value| value.as_bool() == Some(true) || value.as_i64() == Some(1));
    if partial {
        return settled.contains(&identity);
    }
    settled.insert(identity);
    false
}

fn validate_limit(limit: u16) -> Result<(), LedgerError> {
    if limit == 0 || limit > MAX_PAGE_LIMIT {
        return Err(LedgerError::Invariant(
            "projection page limit is invalid".into(),
        ));
    }
    Ok(())
}

fn event(row: &Row<'_>) -> rusqlite::Result<AgentChatProjectionEvent> {
    Ok(AgentChatProjectionEvent {
        cursor: row.get(0)?,
        source_event_id: row.get(1)?,
        kind: row.get(2)?,
        payload: serde_json::from_str(&row.get::<_, String>(3)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
    })
}
