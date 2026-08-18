//! `SQLite` read-only implementation for exact durable turn following.

use gent_ports::{LedgerError, TurnFollowPage, TurnFollowReader};
use gent_types::{
    DurableTurnPhase, HostEpoch, NormalizedTranscriptEvent, NormalizedTranscriptKind, TurnRecord,
};
use rusqlite::{OptionalExtension, params};

use super::{
    SqliteLedger,
    queries::{host_ingress, storage_error},
};

const MAX_PAGE_LIMIT: u16 = 100;

impl TurnFollowReader for SqliteLedger {
    fn turn_follow_host_epoch(&self) -> Result<HostEpoch, LedgerError> {
        Ok(host_ingress(&*self.lock()?)?.epoch)
    }

    fn turn_follow_page(
        &self,
        conversation_id: &str,
        run_id: &str,
        turn_id: &str,
        after_cursor: u64,
        limit: u16,
    ) -> Result<TurnFollowPage, LedgerError> {
        if limit == 0 || limit > MAX_PAGE_LIMIT {
            return Err(LedgerError::Invariant(
                "turn follow page limit is invalid".into(),
            ));
        }
        let after = i64::try_from(after_cursor)
            .map_err(|_| LedgerError::Invariant("transcript cursor exceeds SQLite range".into()))?;
        let connection = self.lock()?;
        let turn = connection
            .query_row(
                "SELECT turn_id, conversation_id, run_id, sequence, phase FROM turns WHERE conversation_id = ?1 AND run_id = ?2 AND turn_id = ?3",
                params![conversation_id, run_id, turn_id], decode_turn,
            )
            .optional()
            .map_err(storage_error)?
            .ok_or_else(|| LedgerError::Invariant("turn follow hierarchy is unknown".into()))?;
        let mut statement = connection
            .prepare(
                "SELECT cursor, event_id, turn_id, run_id, kind, text, is_partial FROM agent_chat_transcript_events WHERE conversation_id = ?1 AND run_id = ?2 AND turn_id = ?3 AND cursor > ?4 ORDER BY cursor ASC LIMIT ?5",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(
                params![
                    conversation_id,
                    run_id,
                    turn_id,
                    after,
                    i64::from(limit) + 1
                ],
                decode_event,
            )
            .map_err(storage_error)?;
        let mut events = rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)?;
        let has_next = events.len() > usize::from(limit);
        events.truncate(usize::from(limit));
        Ok(TurnFollowPage {
            turn,
            next_after_cursor: has_next
                .then(|| events.last().map_or(after_cursor, |event| event.cursor)),
            events,
        })
    }
}

fn decode_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnRecord> {
    Ok(TurnRecord {
        turn_id: row.get(0)?,
        conversation_id: row.get(1)?,
        run_id: row.get(2)?,
        sequence: row.get(3)?,
        phase: decode_phase(&row.get::<_, String>(4)?)?,
    })
}

fn decode_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<NormalizedTranscriptEvent> {
    Ok(NormalizedTranscriptEvent {
        cursor: row.get(0)?,
        event_id: row.get(1)?,
        turn_id: row.get(2)?,
        run_id: row.get(3)?,
        kind: decode_kind(&row.get::<_, String>(4)?)?,
        text: row.get(5)?,
        is_partial: row.get::<_, i64>(6)? != 0,
    })
}

fn decode_phase(value: &str) -> rusqlite::Result<DurableTurnPhase> {
    match value {
        "active" => Ok(DurableTurnPhase::Active),
        "waitingPermission" => Ok(DurableTurnPhase::WaitingPermission),
        "waitingQuestion" => Ok(DurableTurnPhase::WaitingQuestion),
        "completed" => Ok(DurableTurnPhase::Completed),
        "interrupted" => Ok(DurableTurnPhase::Interrupted),
        "failed" => Ok(DurableTurnPhase::Failed),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn decode_kind(value: &str) -> rusqlite::Result<NormalizedTranscriptKind> {
    match value {
        "userMessage" => Ok(NormalizedTranscriptKind::UserMessage),
        "assistantMessage" => Ok(NormalizedTranscriptKind::AssistantMessage),
        "toolActivity" => Ok(NormalizedTranscriptKind::ToolActivity),
        "notice" => Ok(NormalizedTranscriptKind::Notice),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
