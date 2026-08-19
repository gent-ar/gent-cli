//! Authority-gated persistence and paging of canonical conversation activity facts.

use gent_core::{activity_scope, validate_conversation_activity_fact};
use gent_ports::{
    ConversationActivityLedger, IngressMode, Ledger, MAX_CONVERSATION_ACTIVITY_PAGE_FACTS,
};
use gent_types::{ConversationActivityFact, ConversationActivityPage, ConversationActivityScope};

use crate::RuntimeError;

/// Explicit boundary for activity facts that could otherwise drive a client UI.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConversationActivityAuthority {
    /// Shipped observer behavior: do not inspect, persist, or serve activity facts.
    #[default]
    Observer,
    /// Reserved for a separately approved single coordinator with authoritative fact ingress.
    Approved,
}

/// Result of recording one immutable activity fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationActivityResult {
    DeniedObserver,
    Recorded(ConversationActivityFact),
}

/// A bounded activity-history response without replacement state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationActivityRead {
    DeniedObserver,
    Page(ConversationActivityPage),
}

/// Persists typed facts only for an explicitly approved coordinator.
#[derive(Clone, Debug)]
pub struct ConversationActivityService<L> {
    ledger: L,
    authority: ConversationActivityAuthority,
}

impl<L> ConversationActivityService<L> {
    /// Builds an activity service. Shipped daemon composition must use `Observer`.
    #[must_use]
    pub fn new(ledger: L, authority: ConversationActivityAuthority) -> Self {
        Self { ledger, authority }
    }
}

impl<L: Ledger + ConversationActivityLedger> ConversationActivityService<L> {
    /// Appends one assigned, immutable fact only for an approved coordinator.
    ///
    /// # Errors
    /// Returns an error when host fencing or durable fact persistence rejects the fact.
    pub fn record(
        &self,
        fact: &ConversationActivityFact,
    ) -> Result<ConversationActivityResult, RuntimeError> {
        if self.authority != ConversationActivityAuthority::Approved {
            return Ok(ConversationActivityResult::DeniedObserver);
        }
        validate_conversation_activity_fact(fact)
            .map_err(gent_ports::LedgerError::Invariant)
            .map_err(RuntimeError::Ledger)?;
        let scope = activity_scope(fact);
        require_open_host(&self.ledger, scope)?;
        self.ledger.append_conversation_activity(fact)?;
        Ok(ConversationActivityResult::Recorded(fact.clone()))
    }

    /// Reads a bounded, ordered page of immutable facts.
    ///
    /// # Errors
    /// Returns an error when durable activity data cannot be read.
    pub fn read(
        &self,
        conversation_id: &str,
        run_id: &str,
        after_cursor: u64,
    ) -> Result<ConversationActivityRead, RuntimeError> {
        if self.authority != ConversationActivityAuthority::Approved {
            return Ok(ConversationActivityRead::DeniedObserver);
        }
        Ok(ConversationActivityRead::Page(
            self.ledger.read_conversation_activity_page(
                conversation_id,
                run_id,
                after_cursor,
                MAX_CONVERSATION_ACTIVITY_PAGE_FACTS,
            )?,
        ))
    }
}

fn require_open_host<L: Ledger>(
    ledger: &L,
    scope: &ConversationActivityScope,
) -> Result<(), RuntimeError> {
    let ingress = ledger.host_ingress()?;
    if ingress.epoch != scope.host_epoch {
        return Err(RuntimeError::Ledger(gent_ports::LedgerError::StaleEpoch {
            command: scope.host_epoch,
            active: ingress.epoch,
        }));
    }
    if ingress.mode == IngressMode::Closed {
        return Err(RuntimeError::Ledger(
            gent_ports::LedgerError::IngressClosed {
                epoch: ingress.epoch,
            },
        ));
    }
    Ok(())
}
