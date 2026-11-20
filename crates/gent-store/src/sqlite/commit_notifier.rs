use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::Instant,
};

use rusqlite::Connection;
use tokio::sync::watch;

#[derive(Debug)]
struct Commits {
    watch: watch::Sender<u64>,
    count: Mutex<u64>,
    committed: Condvar,
}

#[derive(Clone, Debug)]
pub(super) struct CommitNotifier(Arc<Commits>);

impl CommitNotifier {
    pub(super) fn new() -> Self {
        Self(Arc::new(Commits {
            watch: watch::channel(0).0,
            count: Mutex::new(0),
            committed: Condvar::new(),
        }))
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.watch.subscribe()
    }

    pub(super) fn count(&self) -> u64 {
        *self
            .0
            .count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn wait_past(&self, seen: &mut u64, deadline: Instant) -> bool {
        let mut count = self
            .0
            .count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *count == *seen {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            count = self
                .0
                .committed
                .wait_timeout(count, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        *seen = *count;
        true
    }

    fn notify(&self) {
        let mut count = self
            .0
            .count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *count += 1;
        self.0.watch.send_replace(*count);
        self.0.committed.notify_all();
    }

    pub(super) fn guard<'a>(&'a self, connection: MutexGuard<'a, Connection>) -> LedgerGuard<'a> {
        LedgerGuard {
            changes: connection.total_changes(),
            connection,
            commits: self,
        }
    }
}

pub(crate) struct LedgerGuard<'a> {
    connection: MutexGuard<'a, Connection>,
    changes: u64,
    commits: &'a CommitNotifier,
}

impl Deref for LedgerGuard<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.connection
    }
}

impl DerefMut for LedgerGuard<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

impl Drop for LedgerGuard<'_> {
    fn drop(&mut self) {
        if self.connection.total_changes() != self.changes {
            self.commits.notify();
        }
    }
}

#[cfg(test)]
#[path = "commit_notifier_tests.rs"]
mod tests;
