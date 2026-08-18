//! Owned provider activity ingress with durable-source-before-projection ordering.

use gent_ports::{ConversationActivityLedger, IngressMode, Ledger, RunProjectionLedger};
use gent_types::{
    ConversationActivityFact, ConversationActivityScope, Event, HostEpoch, ReceiptId,
};

use crate::{
    ConversationActivityResult, ConversationActivityService, Coordinator, ProviderRunAuthority,
    RuntimeError,
};

/// A content-free activity input reported by an owned provider runner.
///
/// Its scope carries immutable conversation, run, turn, and epoch identity. `cursor` must be
/// zero: the ingress substitutes the cursor allocated by the durable source event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderActivityFact {
    pub event_id: String,
    pub activity: ConversationActivityFact,
}

/// Bridges an owned provider fact to the conversation activity projection.
///
/// This future-authority component is deliberately independent from daemon composition. It
/// rejects observer mode before it writes either source events or activity checkpoints.
#[derive(Debug)]
pub struct ProviderActivityIngress<L> {
    coordinator: Coordinator<L>,
    activity: ConversationActivityService<L>,
    authority: ProviderRunAuthority,
}

impl<L> ProviderActivityIngress<L>
where
    L: Clone + Ledger + RunProjectionLedger + ConversationActivityLedger,
{
    /// Creates an inert ingress unless explicit public-driver authority is supplied.
    #[must_use]
    pub fn new(
        coordinator: Coordinator<L>,
        activity: ConversationActivityService<L>,
        authority: ProviderRunAuthority,
    ) -> Self {
        Self {
            coordinator,
            activity,
            authority,
        }
    }

    /// Persists one owned activity source, then applies its source cursor to the projection.
    ///
    /// A retry finds the original source event and reapplies its exact cursor, making a partial
    /// source-before-projection failure recoverable without allowing caller-selected ordering.
    ///
    /// # Errors
    /// Returns an error when authority, run ownership, source identity, or persistence fails.
    pub fn record(
        &self,
        coordinator_id: &str,
        input: ProviderActivityFact,
    ) -> Result<ConversationActivityResult, RuntimeError> {
        let scope = scope(&input.activity);
        self.require_owner(&scope.run_id, coordinator_id, scope.host_epoch)?;
        validate_input(&input)?;
        let proposed = source_event(&input);
        let source = match self.coordinator.ledger.find_event(&proposed.event_id)? {
            Some(existing) if same_source(&existing, &proposed) => existing,
            Some(_) => return Err(invariant("provider activity event id was reused")),
            None => self.coordinator.ledger.append_event(&proposed)?,
        };
        match self
            .activity
            .record(&with_cursor(input.activity, source.cursor))?
        {
            ConversationActivityResult::DeniedObserver => {
                Err(invariant("provider activity service is not approved"))
            }
            result => Ok(result),
        }
    }

    fn require_owner(
        &self,
        run_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        if !matches!(
            self.authority,
            ProviderRunAuthority::PublicDrivers | ProviderRunAuthority::PrivateClaurstBridge
        ) {
            return Err(invariant(
                "observer mode cannot accept provider activity facts",
            ));
        }
        let ingress = self.coordinator.ledger.host_ingress()?;
        if ingress.epoch != host_epoch || ingress.mode == IngressMode::Closed {
            return Err(invariant("provider activity source is fenced or closed"));
        }
        let owned = self
            .coordinator
            .ledger
            .find_run_lease(run_id)?
            .is_some_and(|lease| {
                lease.coordinator_id == coordinator_id && lease.host_epoch == host_epoch
            });
        if !owned {
            return Err(invariant("provider activity reporter does not own the run"));
        }
        if self
            .coordinator
            .ledger
            .find_run_session_binding(run_id)?
            .is_none()
        {
            return Err(invariant(
                "provider activity requires a daemon-owned session",
            ));
        }
        Ok(())
    }
}

fn validate_input(input: &ProviderActivityFact) -> Result<(), RuntimeError> {
    let scope = scope(&input.activity);
    if input.event_id.trim().is_empty()
        || scope.conversation_id.trim().is_empty()
        || scope.run_id.trim().is_empty()
        || scope.turn_id.trim().is_empty()
    {
        return Err(invariant("provider activity identity is required"));
    }
    if scope.cursor != 0 {
        return Err(invariant(
            "provider activity cursor is allocated by its durable source",
        ));
    }
    Ok(())
}

fn source_event(input: &ProviderActivityFact) -> Event {
    let scope = scope(&input.activity);
    Event {
        cursor: 0,
        event_id: input.event_id.clone(),
        receipt_id: ReceiptId(format!("providerActivity:{}", scope.run_id)),
        host_epoch: scope.host_epoch,
        kind: "providerActivity".into(),
        payload: serde_json::json!({
            "conversationId": scope.conversation_id,
            "runId": scope.run_id,
            "turnId": scope.turn_id,
            "activity": input.activity,
        }),
    }
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

fn with_cursor(mut fact: ConversationActivityFact, cursor: u64) -> ConversationActivityFact {
    match &mut fact {
        ConversationActivityFact::TurnStarted { scope }
        | ConversationActivityFact::RootActivity { scope, .. }
        | ConversationActivityFact::RootPhase { scope, .. }
        | ConversationActivityFact::WorkPhase { scope, .. }
        | ConversationActivityFact::DecisionPending { scope, .. }
        | ConversationActivityFact::DecisionSettled { scope, .. }
        | ConversationActivityFact::InterruptRequested { scope }
        | ConversationActivityFact::Recovered { scope }
        | ConversationActivityFact::Terminal { scope, .. } => scope.cursor = cursor,
    }
    fact
}

fn same_source(existing: &Event, proposed: &Event) -> bool {
    existing.receipt_id == proposed.receipt_id
        && existing.host_epoch == proposed.host_epoch
        && existing.kind == proposed.kind
        && existing.payload == proposed.payload
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}
