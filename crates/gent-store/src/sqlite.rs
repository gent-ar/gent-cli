use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gent_ports::{
    HostIngress, IngressMode, LeaseClaim, Ledger, LedgerError, ReceiptClaim, RunLease,
    RunLeaseClaim, RunRecord, WorktreeLease,
};
use gent_types::{Command, Event, HostEpoch, Receipt, ReceiptStatus, RunVersionLock};
use rusqlite::{Connection, params};

mod leases;
mod queries;
use queries::{
    append_event, encode_status, events_after, find_lease, find_receipt, find_run,
    find_run_version_lock, host_ingress, insert_lease, insert_receipt, replace_lease,
    save_run_version_lock, storage_error,
};

#[derive(Clone, Debug)]
pub struct SqliteLedger {
    connection: Arc<Mutex<Connection>>,
}

impl SqliteLedger {
    /// Opens or creates a durable ledger at `path`.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot open or migrate the database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LedgerError> {
        Self::from_connection(Connection::open(path).map_err(storage_error)?)
    }
    /// Creates an isolated in-memory ledger for deterministic tests.
    ///
    /// # Errors
    /// Returns an error when `SQLite` cannot initialize the database.
    pub fn in_memory() -> Result<Self, LedgerError> {
        Self::from_connection(Connection::open_in_memory().map_err(storage_error)?)
    }
    fn from_connection(connection: Connection) -> Result<Self, LedgerError> {
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(storage_error)?;
        let ledger = Self {
            connection: Arc::new(Mutex::new(connection)),
        };
        ledger.initialize()?;
        Ok(ledger)
    }
    fn initialize(&self) -> Result<(), LedgerError> {
        let connection = self.lock()?;
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS host_state (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), epoch INTEGER NOT NULL, ingress TEXT NOT NULL DEFAULT 'open');
            INSERT OR IGNORE INTO host_state (singleton, epoch, ingress) VALUES (1, 1, 'open');
            CREATE TABLE IF NOT EXISTS receipts (idempotency_key TEXT PRIMARY KEY NOT NULL, receipt_id TEXT NOT NULL UNIQUE, status TEXT NOT NULL, host_epoch INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS events (cursor INTEGER PRIMARY KEY AUTOINCREMENT, event_id TEXT NOT NULL UNIQUE, receipt_id TEXT NOT NULL, host_epoch INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS runs (run_id TEXT PRIMARY KEY NOT NULL, parent_run_id TEXT REFERENCES runs(run_id), provider TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS run_version_locks (run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id), provider TEXT NOT NULL, canonical_path TEXT NOT NULL, file_identity TEXT NOT NULL, digest_sha256 TEXT NOT NULL, version TEXT NOT NULL, compatibility_entry TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS run_leases (run_id TEXT PRIMARY KEY NOT NULL REFERENCES runs(run_id), coordinator_id TEXT NOT NULL, host_epoch INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS worktree_leases (worktree_id TEXT PRIMARY KEY NOT NULL, run_id TEXT NOT NULL REFERENCES runs(run_id), lease_token TEXT NOT NULL UNIQUE, host_epoch INTEGER NOT NULL);")
            .map_err(storage_error)?;
        if !has_column(&connection, "host_state", "ingress")? {
            connection
                .execute(
                    "ALTER TABLE host_state ADD COLUMN ingress TEXT NOT NULL DEFAULT 'open'",
                    [],
                )
                .map_err(storage_error)?;
        }
        Ok(())
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, LedgerError> {
        self.connection
            .lock()
            .map_err(|error| LedgerError::Storage(error.to_string()))
    }
}

impl Ledger for SqliteLedger {
    fn host_ingress(&self) -> Result<HostIngress, LedgerError> {
        let connection = self.lock()?;
        host_ingress(&connection)
    }
    fn close_ingress(&self, epoch: HostEpoch) -> Result<HostIngress, LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let state = host_ingress(&transaction)?;
        require_epoch(epoch, state.epoch)?;
        transaction
            .execute(
                "UPDATE host_state SET ingress = 'closed' WHERE singleton = 1",
                [],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(HostIngress {
            epoch,
            mode: IngressMode::Closed,
        })
    }
    fn fence_and_open(&self, epoch: HostEpoch) -> Result<HostIngress, LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let state = host_ingress(&transaction)?;
        require_epoch(epoch, state.epoch)?;
        if state.mode != IngressMode::Closed {
            return Err(LedgerError::Invariant(
                "ingress must be closed before fencing".into(),
            ));
        }
        let next = HostEpoch(
            epoch
                .0
                .checked_add(1)
                .ok_or_else(|| LedgerError::Invariant("host epoch overflow".into()))?,
        );
        transaction
            .execute(
                "UPDATE host_state SET epoch = ?1, ingress = 'open' WHERE singleton = 1",
                [next.0],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(HostIngress {
            epoch: next,
            mode: IngressMode::Open,
        })
    }
    fn claim_command(
        &self,
        command: &Command,
        accepted: &Event,
    ) -> Result<ReceiptClaim, LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let state = host_ingress(&transaction)?;
        require_epoch(command.host_epoch, state.epoch)?;
        if state.mode == IngressMode::Closed {
            return Err(LedgerError::IngressClosed { epoch: state.epoch });
        }
        if let Some(receipt) = find_receipt(&transaction, &command.idempotency_key)? {
            return Ok(ReceiptClaim::Existing(receipt));
        }
        let receipt = Receipt {
            receipt_id: command.receipt_id.clone(),
            idempotency_key: command.idempotency_key.clone(),
            status: ReceiptStatus::Accepted,
            host_epoch: state.epoch,
        };
        insert_receipt(&transaction, &receipt)?;
        append_event(&transaction, accepted)?;
        transaction.commit().map_err(storage_error)?;
        Ok(ReceiptClaim::Accepted(receipt))
    }
    fn settle_receipt(
        &self,
        key: &str,
        status: ReceiptStatus,
        terminal: &Event,
    ) -> Result<Receipt, LedgerError> {
        if status == ReceiptStatus::Accepted {
            return Err(LedgerError::Invariant(
                "receipt settlement must be terminal".into(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let mut receipt = find_receipt(&transaction, key)?
            .ok_or_else(|| LedgerError::Invariant("cannot settle an unknown receipt".into()))?;
        if receipt.status != ReceiptStatus::Accepted {
            if receipt.status == status {
                return Ok(receipt);
            }
            return Err(LedgerError::Invariant(
                "receipt already has a different terminal state".into(),
            ));
        }
        transaction
            .execute(
                "UPDATE receipts SET status = ?1 WHERE idempotency_key = ?2",
                params![encode_status(&status), key],
            )
            .map_err(storage_error)?;
        append_event(&transaction, terminal)?;
        transaction.commit().map_err(storage_error)?;
        receipt.status = status;
        Ok(receipt)
    }
    fn append_event(&self, event: &Event) -> Result<Event, LedgerError> {
        let connection = self.lock()?;
        append_event(&connection, event)
    }
    fn events_after(&self, cursor: u64) -> Result<Vec<Event>, LedgerError> {
        let connection = self.lock()?;
        events_after(&connection, cursor)
    }
    fn create_run(&self, run: &RunRecord) -> Result<(), LedgerError> {
        let connection = self.lock()?;
        if let Some(parent) = &run.parent_run_id {
            if find_run(&connection, parent)?.is_none() {
                return Err(LedgerError::Invariant("run parent does not exist".into()));
            }
        }
        connection
            .execute(
                "INSERT INTO runs (run_id, parent_run_id, provider) VALUES (?1, ?2, ?3)",
                params![run.run_id, run.parent_run_id, run.provider],
            )
            .map(|_| ())
            .map_err(storage_error)
    }
    fn find_run(&self, run_id: &str) -> Result<Option<RunRecord>, LedgerError> {
        let connection = self.lock()?;
        find_run(&connection, run_id)
    }
    fn save_run_version_lock(
        &self,
        run_id: &str,
        lock: &RunVersionLock,
    ) -> Result<(), LedgerError> {
        let connection = self.lock()?;
        save_run_version_lock(&connection, run_id, lock)
    }
    fn find_run_version_lock(&self, run_id: &str) -> Result<Option<RunVersionLock>, LedgerError> {
        let connection = self.lock()?;
        find_run_version_lock(&connection, run_id)
    }
    fn claim_run_lease(&self, requested: &RunLease) -> Result<RunLeaseClaim, LedgerError> {
        leases::claim_run(self, requested)
    }
    fn find_run_lease(&self, run_id: &str) -> Result<Option<RunLease>, LedgerError> {
        let connection = self.lock()?;
        queries::find_run_lease(&connection, run_id)
    }
    fn claim_worktree_lease(&self, requested: &WorktreeLease) -> Result<LeaseClaim, LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let active = host_ingress(&transaction)?.epoch;
        require_epoch(requested.host_epoch, active)?;
        if find_run(&transaction, &requested.run_id)?.is_none() {
            return Err(LedgerError::Invariant("lease run does not exist".into()));
        }
        let result = match find_lease(&transaction, &requested.worktree_id)? {
            None => {
                insert_lease(&transaction, requested)?;
                LeaseClaim::Acquired(requested.clone())
            }
            Some(existing) if existing.host_epoch == active => LeaseClaim::Contended(existing),
            Some(previous) => {
                replace_lease(&transaction, requested)?;
                LeaseClaim::Recovered {
                    previous,
                    current: requested.clone(),
                }
            }
        };
        transaction.commit().map_err(storage_error)?;
        Ok(result)
    }
    fn find_worktree_lease(&self, id: &str) -> Result<Option<WorktreeLease>, LedgerError> {
        let connection = self.lock()?;
        find_lease(&connection, id)
    }
}

fn has_column(connection: &Connection, table: &str, column: &str) -> Result<bool, LedgerError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(storage_error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(storage_error)?;
    Ok(columns.filter_map(Result::ok).any(|name| name == column))
}
pub(super) fn require_epoch(command: HostEpoch, active: HostEpoch) -> Result<(), LedgerError> {
    if command == active {
        Ok(())
    } else {
        Err(LedgerError::StaleEpoch { command, active })
    }
}
