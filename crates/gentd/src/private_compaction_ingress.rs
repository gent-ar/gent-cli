//! Private durable ingress for normalized provider compaction facts.

use gent_core::{
    AgentChatCompactionEffect, AgentChatCompactionState, reduce_agent_chat_compaction,
};
use gent_drivers::public_protocol::PublicCompactionObservation;
use gent_ports::{AgentChatCompactionLedger, AgentChatReadLedger, IngressMode, Ledger};
use gent_runtime::{
    AgentChatCompactionRecoveryAuthority, AgentChatCompactionRecoveryRequest,
    AgentChatCompactionRecoveryResult, AgentChatCompactionRecoveryService, AgentChatReadService,
    RuntimeError,
};
use gent_types::{
    AgentChatCompactionFact, AgentChatConversationId, AgentChatRunId, AgentChatSelection, Event,
    HostEpoch, ReceiptId,
};

/// Daemon-owned correlation for one normalized compaction fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateCompactionRequest {
    pub(crate) run_id: AgentChatRunId,
    pub(crate) conversation_id: AgentChatConversationId,
    pub(crate) coordinator_id: String,
    pub(crate) host_epoch: HostEpoch,
    pub(crate) selection: AgentChatSelection,
    pub(crate) fact: AgentChatCompactionFact,
}

/// Daemon-owned binding of an ID-free normalized compaction observation.
///
/// The runner never supplies the durable source ID or the Gent prompt turn. Both values come
/// from the active daemon binding immediately before the observation reaches this private edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateCompactionObservationRequest {
    pub(crate) run_id: AgentChatRunId,
    pub(crate) conversation_id: AgentChatConversationId,
    pub(crate) coordinator_id: String,
    pub(crate) host_epoch: HostEpoch,
    pub(crate) selection: AgentChatSelection,
    pub(crate) event_id: String,
    pub(crate) turn_id: String,
    pub(crate) observation: PublicCompactionObservation,
}

/// A durably retained fact, rejected reducer fact, or immutable recovery child.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCompactionResult {
    DeniedObserver,
    Recorded(AgentChatCompactionEffect),
    Recovered(gent_types::AgentChatSelectionSwitched),
}

/// Unadvertised daemon composition over the pure reducer and atomic child-switch ledger.
///
/// The ingress receives no provider-native session value and is unreachable from transport.
#[derive(Debug)]
pub(crate) struct PrivateCompactionIngress<L> {
    ledger: L,
    recovery: AgentChatCompactionRecoveryService<L>,
    authority: AgentChatCompactionRecoveryAuthority,
}

impl<L: Clone> PrivateCompactionIngress<L> {
    #[must_use]
    pub(crate) fn new(ledger: L, authority: AgentChatCompactionRecoveryAuthority) -> Self {
        Self {
            recovery: AgentChatCompactionRecoveryService::new(ledger.clone(), authority),
            ledger,
            authority,
        }
    }
}

impl<
    L: Clone
        + Ledger
        + gent_ports::AgentChatSelectionLedger
        + AgentChatReadLedger
        + AgentChatCompactionLedger,
> PrivateCompactionIngress<L>
{
    /// Converts an already-normalized provider observation using only daemon-owned correlation.
    ///
    /// This remains private and is absent from observer bootstrap and transport. It rejects no
    /// facts itself; all identity, ownership, selection, receipt, and recovery checks remain in
    /// [`Self::record`].
    pub(crate) fn record_observation(
        &mut self,
        request: PrivateCompactionObservationRequest,
    ) -> Result<PrivateCompactionResult, RuntimeError> {
        let fact = match request.observation {
            PublicCompactionObservation::Started => AgentChatCompactionFact::Started {
                event_id: request.event_id,
                turn_id: request.turn_id,
            },
            PublicCompactionObservation::Completed => AgentChatCompactionFact::Completed {
                event_id: request.event_id,
                turn_id: request.turn_id,
            },
            PublicCompactionObservation::Failed { failure } => AgentChatCompactionFact::Failed {
                event_id: request.event_id,
                turn_id: request.turn_id,
                failure,
            },
        };
        self.record(PrivateCompactionRequest {
            run_id: request.run_id,
            conversation_id: request.conversation_id,
            coordinator_id: request.coordinator_id,
            host_epoch: request.host_epoch,
            selection: request.selection,
            fact,
        })
    }

    /// Persists the typed source fact before reducing or reserving any recovery child.
    pub(crate) fn record(
        &mut self,
        request: PrivateCompactionRequest,
    ) -> Result<PrivateCompactionResult, RuntimeError> {
        if self.authority != AgentChatCompactionRecoveryAuthority::Approved {
            return Ok(PrivateCompactionResult::DeniedObserver);
        }
        validate(&request)?;
        self.require_owner(&request)?;
        self.require_current_selection(&request)?;
        let source = source(&request);
        let source = match self.ledger.find_event(&source.event_id)? {
            Some(existing) if same_source(&existing, &source) => existing,
            Some(_) => return Err(invariant("compaction event id was reused")),
            None => self.ledger.append_event(&source)?,
        };
        let effect = self.replay_effect(&request.run_id, &source.event_id)?;
        let result = self.recovery.apply(
            &AgentChatCompactionRecoveryRequest {
                source_event_id: source.event_id.clone(),
                source_cursor: source.cursor,
                host_epoch: request.host_epoch,
                conversation_id: request.conversation_id,
                parent_run_id: request.run_id.clone(),
                selection: request.selection,
            },
            &effect,
        )?;
        Ok(match result {
            AgentChatCompactionRecoveryResult::DeniedObserver => {
                return Err(invariant(
                    "compaction recovery authority disagrees with ingress",
                ));
            }
            AgentChatCompactionRecoveryResult::Ignored => PrivateCompactionResult::Recorded(effect),
            AgentChatCompactionRecoveryResult::Recovered(child) => {
                PrivateCompactionResult::Recovered(child)
            }
        })
    }

    fn require_owner(&self, request: &PrivateCompactionRequest) -> Result<(), RuntimeError> {
        let ingress = self.ledger.host_ingress()?;
        if ingress.epoch != request.host_epoch || ingress.mode == IngressMode::Closed {
            return Err(invariant("compaction source is fenced or closed"));
        }
        let lease = self.ledger.find_run_lease(&request.run_id.0)?;
        if !lease.is_some_and(|lease| {
            lease.coordinator_id == request.coordinator_id && lease.host_epoch == request.host_epoch
        }) {
            return Err(invariant("compaction reporter does not own the run"));
        }
        self.ledger
            .find_run_session_binding(&request.run_id.0)?
            .is_some()
            .then_some(())
            .ok_or_else(|| invariant("compaction requires a daemon-owned session"))
    }

    fn require_current_selection(
        &self,
        request: &PrivateCompactionRequest,
    ) -> Result<(), RuntimeError> {
        let actual = AgentChatReadService::new(self.ledger.clone())
            .run_selection(&request.conversation_id.0, &request.run_id.0)?;
        (actual == request.selection)
            .then_some(())
            .ok_or_else(|| invariant("compaction selection is not the durable run selection"))
    }

    /// Replays only immutable compaction events for one run; no process-local recovery state is
    /// retained across an ingress restart or used as a replacement for the ledger.
    fn replay_effect(
        &self,
        run_id: &AgentChatRunId,
        source_event_id: &str,
    ) -> Result<AgentChatCompactionEffect, RuntimeError> {
        let mut after_cursor = 0;
        let mut state = AgentChatCompactionState::default();
        let mut source_effect = None;
        loop {
            let page = self
                .ledger
                .read_agent_chat_compaction_page(&run_id.0, after_cursor, 128)?;
            for event in page.events {
                let Some(fact) = compaction_fact(&event, run_id)? else {
                    continue;
                };
                let (next, effect) = reduce_agent_chat_compaction(state, event.cursor, &fact);
                state = next;
                if event.event_id == source_event_id {
                    source_effect = Some(effect);
                }
            }
            let Some(next) = page.next_after_cursor else {
                break;
            };
            if next <= after_cursor {
                return Err(invariant("compaction event page cursor did not advance"));
            }
            after_cursor = next;
        }
        source_effect.ok_or_else(|| invariant("compaction source was not replayable"))
    }
}

fn validate(request: &PrivateCompactionRequest) -> Result<(), RuntimeError> {
    if request.run_id.0.trim().is_empty()
        || request.conversation_id.0.trim().is_empty()
        || request.coordinator_id.trim().is_empty()
        || event_id(&request.fact).trim().is_empty()
        || event_id(&request.fact).len() > 256
        || turn_id(&request.fact).trim().is_empty()
    {
        return Err(invariant("compaction identities are required"));
    }
    request
        .selection
        .validate()
        .map_err(|_| invariant("compaction selection is invalid"))
}

fn source(request: &PrivateCompactionRequest) -> Event {
    Event {
        cursor: 0,
        event_id: event_id(&request.fact).into(),
        receipt_id: ReceiptId(format!("provider:{}", request.run_id.0)),
        host_epoch: request.host_epoch,
        kind: "agentChatCompaction".into(),
        payload: serde_json::json!({ "runId": request.run_id.0, "compaction": request.fact }),
    }
}

fn event_id(fact: &AgentChatCompactionFact) -> &str {
    match fact {
        AgentChatCompactionFact::Started { event_id, .. }
        | AgentChatCompactionFact::Completed { event_id, .. }
        | AgentChatCompactionFact::Failed { event_id, .. } => event_id,
    }
}

fn turn_id(fact: &AgentChatCompactionFact) -> &str {
    match fact {
        AgentChatCompactionFact::Started { turn_id, .. }
        | AgentChatCompactionFact::Completed { turn_id, .. }
        | AgentChatCompactionFact::Failed { turn_id, .. } => turn_id,
    }
}

fn same_source(existing: &Event, proposed: &Event) -> bool {
    existing.receipt_id == proposed.receipt_id
        && existing.host_epoch == proposed.host_epoch
        && existing.kind == proposed.kind
        && existing.payload == proposed.payload
}

fn compaction_fact(
    event: &Event,
    expected_run_id: &AgentChatRunId,
) -> Result<Option<AgentChatCompactionFact>, RuntimeError> {
    if event.kind != "agentChatCompaction" {
        return Ok(None);
    }
    let run_id = event
        .payload
        .get("runId")
        .and_then(serde_json::Value::as_str);
    if run_id != Some(expected_run_id.0.as_str()) {
        return Ok(None);
    }
    serde_json::from_value(
        event
            .payload
            .get("compaction")
            .cloned()
            .ok_or_else(|| invariant("compaction event is missing its normalized fact"))?,
    )
    .map(Some)
    .map_err(|_| invariant("compaction event fact is malformed"))
}

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}
