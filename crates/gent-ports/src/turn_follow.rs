//! Read-only durable projection for following exactly one conversation turn.

use gent_types::{HostEpoch, NormalizedTranscriptEvent, TurnRecord};

use crate::LedgerError;

/// One bounded page of transcript entries belonging to one exact durable turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnFollowPage {
    pub turn: TurnRecord,
    pub events: Vec<NormalizedTranscriptEvent>,
    pub next_after_cursor: Option<u64>,
}

/// Read-only storage boundary used by a later turn-follow transport.
///
/// Implementations filter transcript entries by all three durable identities before returning
/// them. A terminal turn must not settle until all of its entries are visible.
pub trait TurnFollowReader: Send + Sync {
    /// Reads the current daemon epoch for an optimistic read fence.
    ///
    /// # Errors
    /// Returns an error when the durable ingress state cannot be read.
    fn turn_follow_host_epoch(&self) -> Result<HostEpoch, LedgerError>;
    /// Reads an exact, cursor-ordered, bounded turn transcript projection.
    ///
    /// # Errors
    /// Returns an error when the tuple is absent or its transcript cannot be read.
    fn turn_follow_page(
        &self,
        conversation_id: &str,
        run_id: &str,
        turn_id: &str,
        after_cursor: u64,
        limit: u16,
    ) -> Result<TurnFollowPage, LedgerError>;
}
