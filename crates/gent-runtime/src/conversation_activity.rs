//! Authority-gated persistence of pure conversation activity projections.

use gent_core::{ConversationActivityProjection, project_conversation_activity};
use gent_ports::{
    ConversationActivityLedger, IngressMode, Ledger, MAX_CONVERSATION_ACTIVITY_RESUME_RECORDS,
};
use gent_types::{ConversationActivity, ConversationActivityFact, ConversationActivityScope};

use crate::RuntimeError;

/// Explicit boundary for activity facts that could otherwise drive a client UI.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConversationActivityAuthority {
    /// Shipped observer behavior: do not inspect, reduce, or persist activity facts.
    #[default]
    Observer,
    /// Reserved for a separately approved single coordinator with authoritative fact ingress.
    Approved,
}

/// Result of recording or resuming content-free activity data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationActivityResult {
    DeniedObserver,
    Unchanged(ConversationActivity),
    Applied(ConversationActivity),
    Resumed(Vec<ConversationActivity>),
}

/// A bounded, cursor-based activity response with an explicit replacement fallback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationActivityRead {
    DeniedObserver,
    Missing,
    Snapshot(ConversationActivity),
    Delta(Vec<ConversationActivity>),
}

/// Reduces typed facts through the pure state machine before append-only persistence.
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
    /// Reduces and saves one cursor-ordered activity fact only for an approved coordinator.
    ///
    /// # Errors
    /// Returns an error when host fencing or append-only activity persistence rejects the fact.
    pub fn record(
        &self,
        fact: &ConversationActivityFact,
    ) -> Result<ConversationActivityResult, RuntimeError> {
        if self.authority != ConversationActivityAuthority::Approved {
            return Ok(ConversationActivityResult::DeniedObserver);
        }
        let scope = scope(fact);
        require_open_host(&self.ledger, scope)?;
        let projection = self
            .ledger
            .find_conversation_activity(&scope.conversation_id, &scope.run_id)?
            .map_or_else(
                || {
                    ConversationActivityProjection::new(
                        scope.conversation_id.clone(),
                        scope.run_id.clone(),
                        scope.host_epoch,
                    )
                },
                ConversationActivityProjection::from_record,
            );
        let update = project_conversation_activity(projection, fact);
        let activity = update.projection.snapshot().clone();
        if !update.applied {
            return Ok(ConversationActivityResult::Unchanged(activity));
        }
        self.ledger
            .save_conversation_activity(&update.projection.record())?;
        Ok(ConversationActivityResult::Applied(activity))
    }

    /// Resumes complete activity projections strictly after one durable cursor.
    ///
    /// # Errors
    /// Returns an error when activity persistence cannot be read.
    pub fn resume(
        &self,
        conversation_id: &str,
        run_id: &str,
        after_cursor: u64,
    ) -> Result<ConversationActivityResult, RuntimeError> {
        if self.authority != ConversationActivityAuthority::Approved {
            return Ok(ConversationActivityResult::DeniedObserver);
        }
        let records =
            self.ledger
                .resume_conversation_activity(conversation_id, run_id, after_cursor)?;
        Ok(ConversationActivityResult::Resumed(
            records.into_iter().map(|record| record.activity).collect(),
        ))
    }

    /// Returns bounded deltas or a replacement snapshot when a cursor may have fallen behind.
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
        let Some(current) = self
            .ledger
            .find_conversation_activity(conversation_id, run_id)?
        else {
            return Ok(ConversationActivityRead::Missing);
        };
        let records =
            self.ledger
                .resume_conversation_activity(conversation_id, run_id, after_cursor)?;
        if records.len() == MAX_CONVERSATION_ACTIVITY_RESUME_RECORDS
            || (records.is_empty() && after_cursor < current.activity.cursor)
        {
            return Ok(ConversationActivityRead::Snapshot(current.activity));
        }
        Ok(ConversationActivityRead::Delta(
            records.into_iter().map(|record| record.activity).collect(),
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

fn scope(fact: &ConversationActivityFact) -> &ConversationActivityScope {
    match fact {
        ConversationActivityFact::TurnStarted { scope }
        | ConversationActivityFact::RootActivity { scope, .. }
        | ConversationActivityFact::RootPhase { scope, .. }
        | ConversationActivityFact::WorkPhase { scope, .. }
        | ConversationActivityFact::DecisionPending { scope, .. }
        | ConversationActivityFact::DecisionSettled { scope, .. }
        | ConversationActivityFact::InterruptRequested { scope }
        | ConversationActivityFact::Recovered { scope }
        | ConversationActivityFact::Terminal { scope, .. } => scope,
    }
}
