use gent_ports::{
    HostIngress, IngressMode, LedgerError, RunLease, RunRecord, RunSessionBinding, WorktreeLease,
};
use gent_types::{Command, Event, HostEpoch, Receipt, ReceiptId, ReceiptStatus, RunVersionLock};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn insert_receipt(
    connection: &Connection,
    receipt: &Receipt,
    command: &Command,
) -> Result<(), LedgerError> {
    connection.execute("INSERT INTO receipts (idempotency_key, receipt_id, status, host_epoch, kind, payload_digest) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![receipt.idempotency_key, receipt.receipt_id.0, encode_status(&receipt.status), receipt.host_epoch.0, command.kind, command_fingerprint(command)?]).map(|_| ()).map_err(storage_error)
}

pub(super) fn receipt_matches_command(
    connection: &Connection,
    command: &Command,
) -> Result<bool, LedgerError> {
    let wanted_fingerprint = command_fingerprint(command)?;
    connection
        .query_row(
            "SELECT receipt_id, host_epoch, kind, payload_digest FROM receipts WHERE idempotency_key = ?1",
            [&command.idempotency_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
        )
        .optional()
        .map_err(storage_error)
        .map(|stored| stored.is_some_and(|(receipt_id, epoch, kind, fingerprint)| {
            receipt_id == command.receipt_id.0
                && epoch == command.host_epoch.0
                && (kind.is_empty() || kind == command.kind)
                && (fingerprint.is_empty() || fingerprint == wanted_fingerprint)
        }))
}

pub(super) fn command_fingerprint(command: &Command) -> Result<String, LedgerError> {
    use sha2::{Digest, Sha256};

    let payload = serde_json::to_vec(&command.payload).map_err(storage_error)?;
    let mut digest = Sha256::new();
    digest.update(command.kind.as_bytes());
    digest.update([0]);
    digest.update(payload);
    Ok(format!("{:x}", digest.finalize()))
}
pub(super) fn append_event(connection: &Connection, event: &Event) -> Result<Event, LedgerError> {
    connection.execute("INSERT INTO events (event_id, receipt_id, host_epoch, kind, payload) VALUES (?1, ?2, ?3, ?4, ?5)", params![event.event_id, event.receipt_id.0, event.host_epoch.0, event.kind, serde_json::to_string(&event.payload).map_err(storage_error)?]).map_err(storage_error)?;
    Ok(Event {
        cursor: u64::try_from(connection.last_insert_rowid()).map_err(storage_error)?,
        ..event.clone()
    })
}
pub(super) fn find_event(
    connection: &Connection,
    event_id: &str,
) -> Result<Option<Event>, LedgerError> {
    connection.query_row("SELECT cursor, event_id, receipt_id, host_epoch, kind, payload FROM events WHERE event_id = ?1", [event_id], |row| Ok(Event {
        cursor: row.get(0)?, event_id: row.get(1)?, receipt_id: ReceiptId(row.get(2)?), host_epoch: HostEpoch(row.get(3)?), kind: row.get(4)?, payload: serde_json::from_str(&row.get::<_, String>(5)?).map_err(json_error)?,
    })).optional().map_err(storage_error)
}
pub(super) fn events_page(
    connection: &Connection,
    cursor: u64,
    limit: usize,
) -> Result<Vec<Event>, LedgerError> {
    let limit = i64::try_from(limit)
        .map_err(|_| LedgerError::Invariant("event page limit exceeds SQLite range".into()))?;
    let mut statement = connection.prepare("SELECT cursor, event_id, receipt_id, host_epoch, kind, payload FROM events WHERE cursor > ?1 ORDER BY cursor ASC LIMIT ?2").map_err(storage_error)?;
    let rows = statement
        .query_map(rusqlite::params![cursor, limit], |row| {
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
pub(super) fn host_ingress(connection: &Connection) -> Result<HostIngress, LedgerError> {
    connection
        .query_row(
            "SELECT epoch, ingress FROM host_state WHERE singleton = 1",
            [],
            |row| {
                Ok(HostIngress {
                    epoch: HostEpoch(row.get(0)?),
                    mode: decode_ingress(&row.get::<_, String>(1)?)?,
                })
            },
        )
        .map_err(storage_error)
}
pub(super) fn find_receipt(
    connection: &Connection,
    key: &str,
) -> Result<Option<Receipt>, LedgerError> {
    connection
        .query_row(
            "SELECT receipt_id, status, host_epoch FROM receipts WHERE idempotency_key = ?1",
            [key],
            |row| {
                Ok(Receipt {
                    receipt_id: ReceiptId(row.get(0)?),
                    idempotency_key: key.into(),
                    status: decode_status(&row.get::<_, String>(1)?)?,
                    host_epoch: HostEpoch(row.get(2)?),
                })
            },
        )
        .optional()
        .map_err(storage_error)
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
pub(super) fn save_run_version_lock(
    connection: &Connection,
    run_id: &str,
    lock: &RunVersionLock,
) -> Result<(), LedgerError> {
    connection
        .execute(
            "INSERT INTO run_version_locks (run_id, provider, canonical_path, file_identity, digest_sha256, version, compatibility_entry) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![run_id, lock.provider, lock.canonical_path, lock.file_identity, lock.digest_sha256, lock.version, lock.compatibility_entry],
        )
        .map(|_| ())
        .map_err(storage_error)
}
pub(super) fn find_run_version_lock(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<RunVersionLock>, LedgerError> {
    connection
        .query_row(
            "SELECT provider, canonical_path, file_identity, digest_sha256, version, compatibility_entry FROM run_version_locks WHERE run_id = ?1",
            [run_id],
            |row| {
                Ok(RunVersionLock {
                    provider: row.get(0)?, canonical_path: row.get(1)?, file_identity: row.get(2)?,
                    digest_sha256: row.get(3)?, version: row.get(4)?, compatibility_entry: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(storage_error)
}
pub(super) fn save_run_session_binding(
    connection: &Connection,
    binding: &RunSessionBinding,
) -> Result<(), LedgerError> {
    let existing = find_run_session_binding(connection, &binding.run_id)?;
    if let Some(existing) = existing {
        return if existing == *binding {
            Ok(())
        } else {
            Err(LedgerError::Invariant(
                "run session binding is immutable".into(),
            ))
        };
    }
    connection
        .execute(
            "INSERT INTO run_session_bindings (run_id, provider_session_id) VALUES (?1, ?2)",
            params![binding.run_id, binding.provider_session_id],
        )
        .map(|_| ())
        .map_err(storage_error)
}
pub(super) fn find_run_session_binding(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<RunSessionBinding>, LedgerError> {
    connection
        .query_row(
            "SELECT run_id, provider_session_id FROM run_session_bindings WHERE run_id = ?1",
            [run_id],
            |row| {
                Ok(RunSessionBinding {
                    run_id: row.get(0)?,
                    provider_session_id: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(storage_error)
}
pub(super) fn find_run_lease(
    connection: &Connection,
    id: &str,
) -> Result<Option<RunLease>, LedgerError> {
    connection
        .query_row(
            "SELECT run_id, coordinator_id, host_epoch FROM run_leases WHERE run_id = ?1",
            [id],
            |row| {
                Ok(RunLease {
                    run_id: row.get(0)?,
                    coordinator_id: row.get(1)?,
                    host_epoch: HostEpoch(row.get(2)?),
                })
            },
        )
        .optional()
        .map_err(storage_error)
}
pub(super) fn insert_run_lease(
    connection: &Connection,
    lease: &RunLease,
) -> Result<(), LedgerError> {
    connection
        .execute(
            "INSERT INTO run_leases (run_id, coordinator_id, host_epoch) VALUES (?1, ?2, ?3)",
            params![lease.run_id, lease.coordinator_id, lease.host_epoch.0],
        )
        .map(|_| ())
        .map_err(storage_error)
}
pub(super) fn replace_run_lease(
    connection: &Connection,
    lease: &RunLease,
) -> Result<(), LedgerError> {
    connection
        .execute(
            "UPDATE run_leases SET coordinator_id = ?1, host_epoch = ?2 WHERE run_id = ?3",
            params![lease.coordinator_id, lease.host_epoch.0, lease.run_id],
        )
        .map(|_| ())
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
