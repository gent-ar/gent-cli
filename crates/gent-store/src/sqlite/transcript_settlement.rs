use gent_ports::LedgerError;
use gent_types::{INTERRUPTED_REPLY_EVENT_PREFIX, MAX_TRANSCRIPT_TEXT_BYTES};
use rusqlite::{Transaction, params};

use super::queries::storage_error;

const STREAMED_KINDS: [&str; 2] = ["assistantMessage", "thinking"];

pub(crate) const SUPERSEDED_PARTIAL: &str = "e.is_partial = 1 AND EXISTS (SELECT 1 FROM agent_chat_transcript_events s WHERE s.turn_id = e.turn_id AND s.kind = e.kind AND s.is_partial = 0 AND s.cursor > e.cursor)";

struct Unfinished {
    conversation_id: String,
    run_id: String,
    event_id: String,
    text: String,
}

pub(super) fn supersede_unfinished_reply(
    transaction: &Transaction<'_>,
    turn_id: &str,
) -> Result<(), LedgerError> {
    for kind in STREAMED_KINDS {
        let trailing = trailing_partials(transaction, turn_id, kind)?;
        let Some(last) = trailing.last() else {
            continue;
        };
        let text = trailing
            .iter()
            .map(|partial| partial.text.as_str())
            .collect::<String>();
        if text.trim().is_empty() {
            continue;
        }
        transaction
            .execute(
                "INSERT INTO agent_chat_transcript_events (conversation_id, cursor, event_id, turn_id, run_id, kind, text, is_partial) VALUES (?1, (SELECT COALESCE(MAX(cursor), 0) + 1 FROM agent_chat_transcript_events WHERE conversation_id = ?1), ?2, ?3, ?4, ?5, ?6, 0)",
                params![
                    last.conversation_id,
                    format!("{INTERRUPTED_REPLY_EVENT_PREFIX}{}", last.event_id),
                    turn_id,
                    last.run_id,
                    kind,
                    gent_types::bounded_text(&text, MAX_TRANSCRIPT_TEXT_BYTES),
                ],
            )
            .map_err(storage_error)?;
    }
    Ok(())
}

fn trailing_partials(
    transaction: &Transaction<'_>,
    turn_id: &str,
    kind: &str,
) -> Result<Vec<Unfinished>, LedgerError> {
    let mut statement = transaction
        .prepare_cached(
            "SELECT conversation_id, run_id, event_id, text FROM agent_chat_transcript_events WHERE turn_id = ?1 AND kind = ?2 AND is_partial = 1 AND cursor > COALESCE((SELECT MAX(cursor) FROM agent_chat_transcript_events WHERE turn_id = ?1 AND kind = ?2 AND is_partial = 0), 0) ORDER BY cursor ASC",
        )
        .map_err(storage_error)?;
    statement
        .query_map(params![turn_id, kind], |row| {
            Ok(Unfinished {
                conversation_id: row.get(0)?,
                run_id: row.get(1)?,
                event_id: row.get(2)?,
                text: row.get(3)?,
            })
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)
}

#[cfg(test)]
#[path = "transcript_settlement_tests.rs"]
mod tests;
