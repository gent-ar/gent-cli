use gent_ports::LedgerError;
use gent_types::{EventResume, EventSnapshot, HostEpoch};
use rusqlite::{Connection, OptionalExtension, params};

use super::queries::{events_after, host_ingress, storage_error};

pub(super) fn resume(connection: &Connection, cursor: u64) -> Result<EventResume, LedgerError> {
    let snapshot = load(connection)?;
    match snapshot {
        Some(snapshot) if cursor < snapshot.cursor => Ok(EventResume::Resync {
            events: events_after(connection, snapshot.cursor)?,
            snapshot,
        }),
        _ => Ok(EventResume::Delta {
            events: events_after(connection, cursor)?,
        }),
    }
}

pub(super) fn compact(
    connection: &mut Connection,
    snapshot: &EventSnapshot,
) -> Result<(), LedgerError> {
    let transaction = connection.transaction().map_err(storage_error)?;
    let active = host_ingress(&transaction)?.epoch;
    if snapshot.host_epoch != active {
        return Err(LedgerError::StaleEpoch {
            command: snapshot.host_epoch,
            active,
        });
    }
    let head = transaction
        .query_row("SELECT COALESCE(MAX(cursor), 0) FROM events", [], |row| {
            row.get(0)
        })
        .map_err(storage_error)?;
    if snapshot.cursor > head {
        return Err(LedgerError::Invariant(
            "event snapshot cannot exceed the durable event head".into(),
        ));
    }
    if let Some(current) = load(&transaction)?
        && snapshot.cursor <= current.cursor
    {
        return Err(LedgerError::Invariant(
            "event snapshot cursor must advance monotonically".into(),
        ));
    }
    transaction
        .execute(
            "INSERT INTO event_snapshots (singleton, cursor, host_epoch, schema_version, payload) VALUES (1, ?1, ?2, ?3, ?4)",
            params![snapshot.cursor, snapshot.host_epoch.0, snapshot.schema_version, serde_json::to_string(&snapshot.payload).map_err(storage_error)?],
        )
        .map_err(storage_error)?;
    transaction
        .execute("DELETE FROM events WHERE cursor <= ?1", [snapshot.cursor])
        .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)
}

fn load(connection: &Connection) -> Result<Option<EventSnapshot>, LedgerError> {
    let row = connection
        .query_row(
            "SELECT cursor, host_epoch, schema_version, payload FROM event_snapshots WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get::<_, String>(3)?)),
        )
        .optional()
        .map_err(storage_error)?;
    row.map(|(cursor, epoch, schema_version, payload)| {
        Ok(EventSnapshot {
            cursor,
            host_epoch: HostEpoch(epoch),
            schema_version,
            payload: serde_json::from_str(&payload).map_err(storage_error)?,
        })
    })
    .transpose()
}
