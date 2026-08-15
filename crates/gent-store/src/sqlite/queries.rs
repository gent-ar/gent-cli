use gent_ports::{IngressMode, LedgerError, RunRecord, WorktreeLease};
use gent_types::{Event, HostEpoch, Receipt, ReceiptId, ReceiptStatus};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn insert_receipt(
    connection: &Connection,
    receipt: &Receipt,
) -> Result<(), LedgerError> {
    connection.execute("INSERT INTO receipts (idempotency_key, receipt_id, status, host_epoch) VALUES (?1, ?2, ?3, ?4)", params![receipt.idempotency_key, receipt.receipt_id.0, encode_status(&receipt.status), receipt.host_epoch.0]).map(|_| ()).map_err(storage_error)
}
pub(super) fn append_event(connection: &Connection, event: &Event) -> Result<Event, LedgerError> {
    connection.execute("INSERT INTO events (event_id, receipt_id, host_epoch, kind, payload) VALUES (?1, ?2, ?3, ?4, ?5)", params![event.event_id, event.receipt_id.0, event.host_epoch.0, event.kind, serde_json::to_string(&event.payload).map_err(storage_error)?]).map_err(storage_error)?;
    Ok(Event {
        cursor: u64::try_from(connection.last_insert_rowid()).map_err(storage_error)?,
        ..event.clone()
    })
}
pub(super) fn events_after(
    connection: &Connection,
    cursor: u64,
) -> Result<Vec<Event>, LedgerError> {
    let mut statement = connection.prepare("SELECT cursor, event_id, receipt_id, host_epoch, kind, payload FROM events WHERE cursor > ?1 ORDER BY cursor ASC").map_err(storage_error)?;
    let rows = statement
        .query_map([cursor], |row| {
            Ok(Event {
                cursor: row.get(0)?,
                event_id: row.get(1)?,
                receipt_id: ReceiptId(row.get(2)?),
                host_epoch: HostEpoch(row.get(3)?),
                kind: row.get(4)?,
                payload: serde_json::from_str(&row.get::<_, String>(5)?).map_err(json_error)?,
            })
        })
        .map_err(storage_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
}
pub(super) fn find_run(
    connection: &Connection,
    id: &str,
) -> Result<Option<RunRecord>, LedgerError> {
    connection
        .query_row(
            "SELECT run_id, parent_run_id, provider FROM runs WHERE run_id = ?1",
            [id],
            |row| {
                Ok(RunRecord {
                    run_id: row.get(0)?,
                    parent_run_id: row.get(1)?,
                    provider: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(storage_error)
}
pub(super) fn find_lease(
    connection: &Connection,
    id: &str,
) -> Result<Option<WorktreeLease>, LedgerError> {
    connection.query_row("SELECT worktree_id, run_id, lease_token, host_epoch FROM worktree_leases WHERE worktree_id = ?1", [id], |row| Ok(WorktreeLease { worktree_id: row.get(0)?, run_id: row.get(1)?, lease_token: row.get(2)?, host_epoch: HostEpoch(row.get(3)?) })).optional().map_err(storage_error)
}
pub(super) fn insert_lease(
    connection: &Connection,
    lease: &WorktreeLease,
) -> Result<(), LedgerError> {
    connection.execute("INSERT INTO worktree_leases (worktree_id, run_id, lease_token, host_epoch) VALUES (?1, ?2, ?3, ?4)", params![lease.worktree_id, lease.run_id, lease.lease_token, lease.host_epoch.0]).map(|_| ()).map_err(storage_error)
}
pub(super) fn replace_lease(
    connection: &Connection,
    lease: &WorktreeLease,
) -> Result<(), LedgerError> {
    connection.execute("UPDATE worktree_leases SET run_id = ?1, lease_token = ?2, host_epoch = ?3 WHERE worktree_id = ?4", params![lease.run_id, lease.lease_token, lease.host_epoch.0, lease.worktree_id]).map(|_| ()).map_err(storage_error)
}
pub(super) fn encode_status(status: &ReceiptStatus) -> &'static str {
    match status {
        ReceiptStatus::Accepted => "accepted",
        ReceiptStatus::Settled => "settled",
        ReceiptStatus::Unprovable => "unprovable",
        ReceiptStatus::Rejected => "rejected",
    }
}
pub(super) fn decode_status(value: &str) -> rusqlite::Result<ReceiptStatus> {
    match value {
        "accepted" => Ok(ReceiptStatus::Accepted),
        "settled" => Ok(ReceiptStatus::Settled),
        "unprovable" => Ok(ReceiptStatus::Unprovable),
        "rejected" => Ok(ReceiptStatus::Rejected),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
pub(super) fn decode_ingress(value: &str) -> rusqlite::Result<IngressMode> {
    match value {
        "open" => Ok(IngressMode::Open),
        "closed" => Ok(IngressMode::Closed),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn json_error(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
}
pub(super) fn storage_error(error: impl std::fmt::Display) -> LedgerError {
    LedgerError::Storage(error.to_string())
}
