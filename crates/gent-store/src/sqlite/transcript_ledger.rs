//! `SQLite` implementation of bounded normalized transcript storage.

use gent_ports::{LedgerError, TranscriptLedger};
use gent_types::{
    AgentChatConversationId, NormalizedTranscriptAppend, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, NormalizedTranscriptPage,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::{SqliteLedger, queries::storage_error};

const MAX_EVENT_TEXT_BYTES: usize = 64 * 1024;
const MAX_PAGE_LIMIT: u16 = 100;

impl TranscriptLedger for SqliteLedger {
    fn append_normalized_transcript(
        &self,
        conversation_id: &AgentChatConversationId,
        append: &NormalizedTranscriptAppend,
    ) -> Result<NormalizedTranscriptEvent, LedgerError> {
        append_event(self, conversation_id, append)
    }

    fn normalized_transcript_page(
        &self,
        conversation_id: &AgentChatConversationId,
        after_cursor: u64,
        limit: u16,
    ) -> Result<NormalizedTranscriptPage, LedgerError> {
        page(self, conversation_id, after_cursor, limit)
    }
}

fn append_event(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    append: &NormalizedTranscriptAppend,
) -> Result<NormalizedTranscriptEvent, LedgerError> {
    validate(conversation_id, append)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    if let Some(event) = find_by_event_id(&transaction, &append.event_id)? {
        return retry_result(event, conversation_id, append);
    }
    require_hierarchy(&transaction, conversation_id, append)?;
    let cursor = next_cursor(&transaction, conversation_id)?;
    transaction
        .execute(
            "INSERT INTO agent_chat_transcript_events (conversation_id, cursor, event_id, turn_id, run_id, kind, text, is_partial) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![conversation_id.0, cursor, append.event_id, append.turn_id, append.run_id, kind(append.kind), append.text, i64::from(append.is_partial)],
        )
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)?;
    event(cursor, append)
}

pub(super) fn page(
    ledger: &SqliteLedger,
    conversation_id: &AgentChatConversationId,
    after_cursor: u64,
    limit: u16,
) -> Result<NormalizedTranscriptPage, LedgerError> {
    if limit == 0 || limit > MAX_PAGE_LIMIT {
        return Err(LedgerError::Invariant(
            "transcript page limit is invalid".into(),
        ));
    }
    let connection = ledger.lock()?;
    if !conversation_exists(&connection, conversation_id)? {
        return Err(LedgerError::Invariant(
            "agent chat conversation does not exist".into(),
        ));
    }
    let after = i64::try_from(after_cursor)
        .map_err(|_| LedgerError::Invariant("transcript cursor exceeds SQLite range".into()))?;
    let mut statement = connection
        .prepare("SELECT cursor, event_id, turn_id, run_id, kind, text, is_partial FROM agent_chat_transcript_events WHERE conversation_id = ?1 AND cursor > ?2 ORDER BY cursor ASC LIMIT ?3")
        .map_err(storage_error)?;
    let rows = statement
        .query_map(
            params![conversation_id.0, after, i64::from(limit) + 1],
            decode_event,
        )
        .map_err(storage_error)?;
    let mut events = rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?;
    let has_next = events.len() > usize::from(limit);
    events.truncate(usize::from(limit));
    let next_after_cursor =
        has_next.then(|| events.last().map_or(after_cursor, |item| item.cursor));
    Ok(NormalizedTranscriptPage {
        conversation_id: conversation_id.0.clone(),
        events,
        next_after_cursor,
    })
}

fn validate(
    conversation_id: &AgentChatConversationId,
    append: &NormalizedTranscriptAppend,
) -> Result<(), LedgerError> {
    if conversation_id.0.trim().is_empty()
        || [&append.event_id, &append.turn_id, &append.run_id]
            .into_iter()
            .any(|value| value.trim().is_empty())
        || append.text.len() > MAX_EVENT_TEXT_BYTES
        || append.text.contains('\0')
    {
        return Err(LedgerError::Invariant("transcript event is invalid".into()));
    }
    Ok(())
}

fn require_hierarchy(
    transaction: &Transaction<'_>,
    conversation_id: &AgentChatConversationId,
    append: &NormalizedTranscriptAppend,
) -> Result<(), LedgerError> {
    let exists = transaction
        .query_row(
            "SELECT 1 FROM turns t JOIN runs r ON r.run_id = t.run_id JOIN agent_chat_conversations c ON c.conversation_id = t.conversation_id WHERE t.conversation_id = ?1 AND t.turn_id = ?2 AND t.run_id = ?3 AND r.conversation_id = c.conversation_id",
            params![conversation_id.0, append.turn_id, append.run_id], |_| Ok(()),
        )
        .optional()
        .map_err(storage_error)?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(LedgerError::Invariant(
            "transcript event hierarchy is unknown".into(),
        ))
    }
}

fn conversation_exists(
    connection: &rusqlite::Connection,
    conversation_id: &AgentChatConversationId,
) -> Result<bool, LedgerError> {
    connection
        .query_row(
            "SELECT 1 FROM agent_chat_conversations WHERE conversation_id = ?1",
            [&conversation_id.0],
            |_| Ok(()),
        )
        .optional()
        .map(|item| item.is_some())
        .map_err(storage_error)
}

fn next_cursor(
    transaction: &Transaction<'_>,
    conversation_id: &AgentChatConversationId,
) -> Result<i64, LedgerError> {
    transaction
        .query_row("SELECT COALESCE(MAX(cursor), 0) + 1 FROM agent_chat_transcript_events WHERE conversation_id = ?1", [&conversation_id.0], |row| row.get(0))
        .map_err(storage_error)
}

fn find_by_event_id(
    transaction: &Transaction<'_>,
    event_id: &str,
) -> Result<Option<(String, NormalizedTranscriptEvent)>, LedgerError> {
    transaction.query_row("SELECT conversation_id, cursor, event_id, turn_id, run_id, kind, text, is_partial FROM agent_chat_transcript_events WHERE event_id = ?1", [event_id], |row| Ok((row.get(0)?, decode_event_at(row, 1)?))).optional().map_err(storage_error)
}

fn retry_result(
    stored: (String, NormalizedTranscriptEvent),
    conversation_id: &AgentChatConversationId,
    append: &NormalizedTranscriptAppend,
) -> Result<NormalizedTranscriptEvent, LedgerError> {
    let (stored_conversation, event) = stored;
    if stored_conversation == conversation_id.0 && event.cursor > 0 && event_matches(&event, append)
    {
        Ok(event)
    } else {
        Err(LedgerError::Invariant(
            "transcript event retry conflicts with durable ownership".into(),
        ))
    }
}

fn event_matches(event: &NormalizedTranscriptEvent, append: &NormalizedTranscriptAppend) -> bool {
    event.event_id == append.event_id
        && event.turn_id == append.turn_id
        && event.run_id == append.run_id
        && event.kind == append.kind
        && event.text == append.text
        && event.is_partial == append.is_partial
}

fn event(
    cursor: i64,
    append: &NormalizedTranscriptAppend,
) -> Result<NormalizedTranscriptEvent, LedgerError> {
    Ok(NormalizedTranscriptEvent {
        cursor: u64::try_from(cursor)
            .map_err(|_| LedgerError::Storage("transcript cursor is invalid".into()))?,
        event_id: append.event_id.clone(),
        turn_id: append.turn_id.clone(),
        run_id: append.run_id.clone(),
        kind: append.kind,
        text: append.text.clone(),
        is_partial: append.is_partial,
    })
}

fn decode_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<NormalizedTranscriptEvent> {
    decode_event_at(row, 0)
}

fn decode_event_at(
    row: &rusqlite::Row<'_>,
    start: usize,
) -> rusqlite::Result<NormalizedTranscriptEvent> {
    Ok(NormalizedTranscriptEvent {
        cursor: row.get(start)?,
        event_id: row.get(start + 1)?,
        turn_id: row.get(start + 2)?,
        run_id: row.get(start + 3)?,
        kind: decode_kind(&row.get::<_, String>(start + 4)?).map_err(to_sql_error)?,
        text: row.get(start + 5)?,
        is_partial: row.get::<_, i64>(start + 6)? != 0,
    })
}

const fn kind(value: NormalizedTranscriptKind) -> &'static str {
    match value {
        NormalizedTranscriptKind::UserMessage => "userMessage",
        NormalizedTranscriptKind::AssistantMessage => "assistantMessage",
        NormalizedTranscriptKind::ToolActivity => "toolActivity",
        NormalizedTranscriptKind::Notice => "notice",
    }
}

fn decode_kind(value: &str) -> Result<NormalizedTranscriptKind, LedgerError> {
    match value {
        "userMessage" => Ok(NormalizedTranscriptKind::UserMessage),
        "assistantMessage" => Ok(NormalizedTranscriptKind::AssistantMessage),
        "toolActivity" => Ok(NormalizedTranscriptKind::ToolActivity),
        "notice" => Ok(NormalizedTranscriptKind::Notice),
        _ => Err(LedgerError::Storage("unknown transcript event kind".into())),
    }
}

fn to_sql_error(error: LedgerError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}
