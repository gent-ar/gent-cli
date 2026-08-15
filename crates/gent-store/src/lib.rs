//! `SQLite` implementation of the durable ledger port.

use std::path::Path;
use std::sync::{Arc, Mutex};

use gent_ports::{Ledger, LedgerError};
use gent_types::{Event, HostEpoch, Receipt, ReceiptId, ReceiptStatus};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone, Debug)]
pub struct SqliteLedger {
    connection: Arc<Mutex<Connection>>,
}

impl SqliteLedger {
    /// Opens or creates a durable ledger at `path`.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot open or initialize the database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LedgerError> {
        let connection = Connection::open(path).map_err(storage_error)?;
        let ledger = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        ledger.initialize()?;
        Ok(ledger)
    }

    /// Creates an isolated in-memory ledger for deterministic tests.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot initialize the database.
    pub fn in_memory() -> Result<Self, LedgerError> {
        let connection = Connection::open_in_memory().map_err(storage_error)?;
        let ledger = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        ledger.initialize()?;
        Ok(ledger)
    }

    fn initialize(&self) -> Result<(), LedgerError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| LedgerError::Storage(error.to_string()))?;
        connection
            .execute_batch(
                "
                PRAGMA journal_mode = WAL;
                CREATE TABLE IF NOT EXISTS host_state (
                  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                  epoch INTEGER NOT NULL
                );
                INSERT OR IGNORE INTO host_state (singleton, epoch) VALUES (1, 1);
                CREATE TABLE IF NOT EXISTS receipts (
                  idempotency_key TEXT PRIMARY KEY NOT NULL,
                  receipt_id TEXT NOT NULL UNIQUE,
                  status TEXT NOT NULL,
                  host_epoch INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS events (
                  cursor INTEGER PRIMARY KEY AUTOINCREMENT,
                  event_id TEXT NOT NULL UNIQUE,
                  receipt_id TEXT NOT NULL,
                  host_epoch INTEGER NOT NULL,
                  kind TEXT NOT NULL,
                  payload TEXT NOT NULL
                );",
            )
            .map_err(storage_error)
    }
}

impl Ledger for SqliteLedger {
    fn current_epoch(&self) -> Result<HostEpoch, LedgerError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| LedgerError::Storage(error.to_string()))?;
        let epoch = connection
            .query_row(
                "SELECT epoch FROM host_state WHERE singleton = 1",
                [],
                |row| row.get::<_, u64>(0),
            )
            .map_err(storage_error)?;
        Ok(HostEpoch(epoch))
    }

    fn find_receipt(&self, idempotency_key: &str) -> Result<Option<Receipt>, LedgerError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| LedgerError::Storage(error.to_string()))?;
        connection
            .query_row(
                "SELECT receipt_id, status, host_epoch FROM receipts WHERE idempotency_key = ?1",
                [idempotency_key],
                |row| {
                    Ok(Receipt {
                        receipt_id: ReceiptId(row.get(0)?),
                        idempotency_key: idempotency_key.to_owned(),
                        status: decode_status(&row.get::<_, String>(1)?)?,
                        host_epoch: HostEpoch(row.get(2)?),
                    })
                },
            )
            .optional()
            .map_err(storage_error)
    }

    fn record_receipt(&self, receipt: &Receipt) -> Result<(), LedgerError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| LedgerError::Storage(error.to_string()))?;
        connection
            .execute(
                "INSERT INTO receipts (idempotency_key, receipt_id, status, host_epoch) VALUES (?1, ?2, ?3, ?4)",
                params![receipt.idempotency_key, receipt.receipt_id.0, encode_status(&receipt.status), receipt.host_epoch.0],
            )
            .map(|_| ())
            .map_err(storage_error)
    }

    fn update_receipt_status(
        &self,
        idempotency_key: &str,
        status: ReceiptStatus,
    ) -> Result<(), LedgerError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| LedgerError::Storage(error.to_string()))?;
        connection
            .execute(
                "UPDATE receipts SET status = ?1 WHERE idempotency_key = ?2",
                params![encode_status(&status), idempotency_key],
            )
            .map(|_| ())
            .map_err(storage_error)
    }

    fn append_event(&self, event: &Event) -> Result<Event, LedgerError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| LedgerError::Storage(error.to_string()))?;
        connection
            .execute(
                "INSERT INTO events (event_id, receipt_id, host_epoch, kind, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![event.event_id, event.receipt_id.0, event.host_epoch.0, event.kind, serde_json::to_string(&event.payload).map_err(storage_error)?],
            )
            .map_err(storage_error)?;
        Ok(Event {
            cursor: u64::try_from(connection.last_insert_rowid()).map_err(storage_error)?,
            ..event.clone()
        })
    }

    fn events_after(&self, cursor: u64) -> Result<Vec<Event>, LedgerError> {
        let connection = self
            .connection
            .lock()
            .map_err(|error| LedgerError::Storage(error.to_string()))?;
        let mut statement = connection
            .prepare("SELECT cursor, event_id, receipt_id, host_epoch, kind, payload FROM events WHERE cursor > ?1 ORDER BY cursor ASC")
            .map_err(storage_error)?;
        let events = statement
            .query_map([cursor], |row| {
                Ok(Event {
                    cursor: row.get(0)?,
                    event_id: row.get(1)?,
                    receipt_id: ReceiptId(row.get(2)?),
                    host_epoch: HostEpoch(row.get(3)?),
                    kind: row.get(4)?,
                    payload: serde_json::from_str(&row.get::<_, String>(5)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            5,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                })
            })
            .map_err(storage_error)?;
        events.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }
}

fn encode_status(status: &ReceiptStatus) -> &'static str {
    match status {
        ReceiptStatus::Accepted => "accepted",
        ReceiptStatus::Settled => "settled",
        ReceiptStatus::Unprovable => "unprovable",
        ReceiptStatus::Rejected => "rejected",
    }
}

fn decode_status(value: &str) -> rusqlite::Result<ReceiptStatus> {
    match value {
        "accepted" => Ok(ReceiptStatus::Accepted),
        "settled" => Ok(ReceiptStatus::Settled),
        "unprovable" => Ok(ReceiptStatus::Unprovable),
        "rejected" => Ok(ReceiptStatus::Rejected),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn storage_error(error: impl std::fmt::Display) -> LedgerError {
    LedgerError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use gent_ports::Ledger;
    use gent_types::{Event, HostEpoch, ReceiptId};
    use serde_json::json;

    use super::SqliteLedger;

    #[test]
    fn events_are_durable_and_cursor_ordered() {
        let ledger = SqliteLedger::in_memory().unwrap();
        for kind in ["first", "second"] {
            ledger
                .append_event(&Event {
                    cursor: 0,
                    event_id: format!("event-{kind}"),
                    receipt_id: ReceiptId::new(),
                    host_epoch: HostEpoch(1),
                    kind: kind.into(),
                    payload: json!({}),
                })
                .unwrap();
        }
        let events = ledger.events_after(0).unwrap();
        assert_eq!(
            events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(events[0].cursor, 1);
        assert_eq!(events[1].cursor, 2);
    }
}
