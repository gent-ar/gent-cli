use gent_ports::LedgerError;
use gent_types::EventPage;
use rusqlite::{Connection, params};

use super::queries::events_page;

pub(super) const MAX_EVENT_PAGE_SIZE: usize = 100;

pub(super) fn read(
    connection: &Connection,
    after_cursor: u64,
    limit: usize,
) -> Result<EventPage, LedgerError> {
    let limit = limit.clamp(1, MAX_EVENT_PAGE_SIZE);
    let mut events = events_page(connection, after_cursor, limit.saturating_add(1))?;
    let has_more = events.len() > limit;
    events.truncate(limit);
    let next_after_cursor =
        has_more.then(|| events.last().map_or(after_cursor, |event| event.cursor));
    Ok(EventPage {
        events,
        next_after_cursor,
    })
}

pub(super) fn read_compaction(
    connection: &Connection,
    run_id: &str,
    after_cursor: u64,
    limit: usize,
) -> Result<EventPage, LedgerError> {
    let limit = limit.clamp(1, MAX_EVENT_PAGE_SIZE);
    let mut statement = connection
        .prepare(
            "SELECT cursor, event_id, receipt_id, host_epoch, kind, payload FROM events \
             WHERE kind = 'agentChatCompaction' AND json_extract(payload, '$.runId') = ?1 \
             AND cursor > ?2 ORDER BY cursor ASC LIMIT ?3",
        )
        .map_err(super::queries::storage_error)?;
    let rows = statement
        .query_map(
            params![run_id, after_cursor, limit.saturating_add(1)],
            |row| {
                Ok(gent_types::Event {
                    cursor: row.get(0)?,
                    event_id: row.get(1)?,
                    receipt_id: gent_types::ReceiptId(row.get(2)?),
                    host_epoch: gent_types::HostEpoch(row.get(3)?),
                    kind: row.get(4)?,
                    payload: serde_json::from_str(&row.get::<_, String>(5)?).map_err(json_error)?,
                })
            },
        )
        .map_err(super::queries::storage_error)?;
    let mut events = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(super::queries::storage_error)?;
    let has_more = events.len() > limit;
    events.truncate(limit);
    Ok(EventPage {
        next_after_cursor: has_more
            .then(|| events.last().map_or(after_cursor, |event| event.cursor)),
        events,
    })
}

fn json_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
}
