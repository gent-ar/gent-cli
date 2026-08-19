//! Daemon-owned wall-clock boundary for time-sensitive authority decisions.
//!
//! Clients never supply this value. Private composition injects a clock so tests can prove that
//! a long-lived daemon rechecks expiring authority at the effect boundary.

use std::time::{SystemTime, UNIX_EPOCH};

/// Supplies the current authority time for one daemon-owned decision.
pub(crate) trait AuthorityClock: Clone + Send + Sync {
    /// Returns seconds since the Unix epoch, failing closed to zero if system time predates it.
    fn now_unix_seconds(&self) -> u64;
}

/// Production wall clock for an approved daemon composition.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemAuthorityClock;

impl AuthorityClock for SystemAuthorityClock {
    fn now_unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs())
    }
}
