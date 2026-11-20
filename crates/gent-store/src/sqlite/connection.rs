use std::{path::Path, sync::Arc, sync::Mutex, time::Duration};

use rusqlite::Connection;

use super::{
    LedgerError, SqliteLedger,
    commit_notifier::{CommitNotifier, LedgerGuard},
    fresh_schema,
};

pub(super) fn storage_error(error: impl std::fmt::Display + 'static) -> LedgerError {
    let code = (&error as &dyn std::any::Any)
        .downcast_ref::<rusqlite::Error>()
        .and_then(rusqlite::Error::sqlite_error_code);
    match code {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            LedgerError::Busy(error.to_string())
        }
        _ => LedgerError::Storage(error.to_string()),
    }
}

impl SqliteLedger {
    pub fn read_host_state(
        path: impl AsRef<Path>,
    ) -> Result<Option<gent_ports::HostIngress>, LedgerError> {
        let path = path.as_ref();
        if !path.try_exists().map_err(storage_error)? {
            return Ok(None);
        }
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(storage_error)?;
        super::queries::host_ingress(&connection).map(Some)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, LedgerError> {
        Self::from_connection(Connection::open(path).map_err(storage_error)?)
    }

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
        fresh_schema::apply(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            commits: CommitNotifier::new(),
        })
    }

    pub fn commits(&self) -> tokio::sync::watch::Receiver<u64> {
        self.commits.subscribe()
    }

    #[must_use]
    pub fn commit_count(&self) -> u64 {
        self.commits.count()
    }

    pub fn wait_for_commit(&self, seen: &mut u64, deadline: std::time::Instant) -> bool {
        self.commits.wait_past(seen, deadline)
    }

    pub(super) fn lock(&self) -> Result<LedgerGuard<'_>, LedgerError> {
        self.connection
            .lock()
            .map(|connection| self.commits.guard(connection))
            .map_err(|error| LedgerError::Storage(error.to_string()))
    }
}

#[cfg(test)]
mod runtime_owner_reads {
    use super::*;
    use gent_ports::Ledger;

    #[test]
    fn offline_owner_read_neither_creates_nor_migrates_a_ledger() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing.db");
        assert!(SqliteLedger::read_host_state(&missing).unwrap().is_none());
        assert!(!missing.exists());
        let unrelated = root.path().join("unrelated.db");
        let connection = Connection::open(&unrelated).unwrap();
        connection
            .execute_batch("CREATE TABLE unrelated(value TEXT)")
            .unwrap();
        drop(connection);
        assert!(SqliteLedger::read_host_state(&unrelated).is_err());
        let connection = Connection::open(&unrelated).unwrap();
        let count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='gent_schema'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn a_write_blocked_by_another_connection_is_busy_not_storage_failure() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("gent.db");
        let ledger = SqliteLedger::open(&path).unwrap();
        let holder = Connection::open(&path).unwrap();
        holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let error = ledger.close_ingress(gent_types::HostEpoch(1)).unwrap_err();
        assert!(matches!(error, LedgerError::Busy(_)), "{error:?}");
        holder.execute_batch("ROLLBACK").unwrap();
        ledger.close_ingress(gent_types::HostEpoch(1)).unwrap();
    }

    #[test]
    fn offline_owner_read_observes_the_latest_fence_without_writing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("gent.db");
        let ledger = SqliteLedger::open(&path).unwrap();
        ledger.close_ingress(gent_types::HostEpoch(1)).unwrap();
        let expected = ledger.host_ingress().unwrap();
        assert_eq!(
            SqliteLedger::read_host_state(&path).unwrap(),
            Some(expected)
        );
        assert_eq!(ledger.host_ingress().unwrap(), expected);
    }
}
