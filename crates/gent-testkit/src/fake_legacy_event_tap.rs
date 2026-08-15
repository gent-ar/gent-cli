//! Deterministic read-only legacy-tap fake for synthetic observer tests.

use gent_ports::{LegacyEventTap, PortError};
use gent_types::LegacyLifecycleTap;

/// In-memory ordered tap records. It never persists or mutates a production ledger.
#[derive(Clone, Debug, Default)]
pub struct FakeLegacyEventTap {
    records: Vec<LegacyLifecycleTap>,
}

impl FakeLegacyEventTap {
    /// Builds a fixture tap from records already ordered by their legacy cursor.
    #[must_use]
    pub fn new(records: Vec<LegacyLifecycleTap>) -> Self {
        Self { records }
    }
}

impl LegacyEventTap for FakeLegacyEventTap {
    fn read_after(&self, cursor: u64) -> Result<Vec<LegacyLifecycleTap>, PortError> {
        Ok(self
            .records
            .iter()
            .filter(|record| record.cursor > cursor)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use gent_ports::LegacyEventTap;
    use gent_types::{
        ConversationLiveStatus, LegacyLifecycleTap, NormalizedLifecycleSignal, ReceiptId,
    };

    use super::FakeLegacyEventTap;

    #[test]
    fn replay_is_read_only_and_strictly_after_the_requested_cursor() {
        let tap = FakeLegacyEventTap::new(
            [1, 2]
                .into_iter()
                .map(|cursor| LegacyLifecycleTap {
                    cursor,
                    event_id: format!("event-{cursor}"),
                    receipt_id: ReceiptId(format!("receipt-{cursor}")),
                    signal: NormalizedLifecycleSignal::AttentionCleared,
                    reported: ConversationLiveStatus {
                        snapshot_cursor: cursor,
                        ..ConversationLiveStatus::default()
                    },
                })
                .collect(),
        );
        assert_eq!(
            tap.read_after(1)
                .unwrap()
                .into_iter()
                .map(|record| record.cursor)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }
}
