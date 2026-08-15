//! Ephemeral legacy-tap polling. It owns no ledger, socket, process, or mutation capability.

use gent_core::ObserverProjection;
use gent_ports::LegacyEventTap;
use gent_types::ObserverDiagnostic;

use crate::RuntimeError;

/// Poll result for an observer sidecar. A diagnostic blocks later projection advancement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObserverPoll {
    pub cursor: u64,
    pub diagnostics: Vec<ObserverDiagnostic>,
    pub blocked: bool,
}

/// Read-only observer coordinator over an app-owned legacy event tap.
#[derive(Clone, Debug)]
pub struct LegacyObserver<T> {
    tap: T,
    projection: ObserverProjection,
    blocked: bool,
}

impl<T: LegacyEventTap> LegacyObserver<T> {
    #[must_use]
    pub fn new(tap: T) -> Self {
        Self {
            tap,
            projection: ObserverProjection::default(),
            blocked: false,
        }
    }

    /// Applies the ordered readable prefix and stops at the first unexplained divergence.
    ///
    /// # Errors
    /// Returns only the tap adapter error; it cannot create a receipt or write durable state.
    pub fn poll(&mut self) -> Result<ObserverPoll, RuntimeError> {
        if self.blocked {
            return Ok(self.result(Vec::new()));
        }
        let cursor = self.projection.status().snapshot_cursor;
        let mut diagnostics = Vec::new();
        for tap in self.tap.read_after(cursor)? {
            let comparison = self.projection.clone().compare(&tap);
            self.projection = comparison.projection;
            if let Some(diagnostic) = comparison.diagnostic {
                diagnostics.push(diagnostic);
                self.blocked = true;
                break;
            }
        }
        Ok(self.result(diagnostics))
    }

    fn result(&self, diagnostics: Vec<ObserverDiagnostic>) -> ObserverPoll {
        ObserverPoll {
            cursor: self.projection.status().snapshot_cursor,
            diagnostics,
            blocked: self.blocked,
        }
    }
}

#[cfg(test)]
mod tests {
    use gent_ports::{LegacyEventTap, PortError};
    use gent_types::{
        ConversationLiveStatus, LegacyLifecycleTap, NormalizedLifecycleSignal, ReceiptId,
    };

    use super::LegacyObserver;

    #[derive(Clone, Debug)]
    struct Tap(Vec<LegacyLifecycleTap>);

    impl LegacyEventTap for Tap {
        fn read_after(&self, cursor: u64) -> Result<Vec<LegacyLifecycleTap>, PortError> {
            Ok(self
                .0
                .iter()
                .filter(|tap| tap.cursor > cursor)
                .cloned()
                .collect())
        }
    }

    fn tap(cursor: u64, reported: ConversationLiveStatus) -> LegacyLifecycleTap {
        LegacyLifecycleTap {
            cursor,
            event_id: format!("event-{cursor}"),
            receipt_id: ReceiptId(format!("receipt-{cursor}")),
            signal: NormalizedLifecycleSignal::AttentionCleared,
            reported,
        }
    }

    #[test]
    fn divergence_blocks_future_reads_without_writing_a_ledger() {
        let matching = ConversationLiveStatus {
            snapshot_cursor: 1,
            ..ConversationLiveStatus::default()
        };
        let observer = Tap(vec![
            tap(1, matching),
            tap(2, ConversationLiveStatus::default()),
        ]);
        let mut service = LegacyObserver::new(observer);
        let result = service.poll().unwrap();
        assert!(result.blocked);
        assert_eq!(result.cursor, 2);
        assert_eq!(result.diagnostics.len(), 1);
        assert!(service.poll().unwrap().diagnostics.is_empty());
    }
}
