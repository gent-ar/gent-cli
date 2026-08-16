//! `SQLite` connection construction and synchronized access.

use std::{path::Path, sync::Arc, sync::Mutex, time::Duration};

use rusqlite::Connection;

use super::{LedgerError, SqliteLedger, migrations, queries::storage_error};

impl SqliteLedger {
    /// Opens a migrated durable `SQLite` ledger.
    ///
    /// # Errors
    /// Returns an error when the connection or required migration fails.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LedgerError> {
        Self::from_connection(Connection::open(path).map_err(storage_error)?)
    }

    /// Opens an isolated migrated `SQLite` ledger for deterministic tests.
    ///
    /// # Errors
    /// Returns an error when the connection or required migration fails.
    pub fn in_memory() -> Result<Self, LedgerError> {
        Self::from_connection(Connection::open_in_memory().map_err(storage_error)?)
    }

    fn from_connection(mut connection: Connection) -> Result<Self, LedgerError> {
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(storage_error)?;
        connection
            .execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
            .map_err(storage_error)?;
        migrations::apply(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub(super) fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, LedgerError> {
        self.connection
            .lock()
            .map_err(|error| LedgerError::Storage(error.to_string()))
    }
}
