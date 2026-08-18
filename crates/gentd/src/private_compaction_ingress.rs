//! Private durable ingress for normalized provider compaction facts.

use std::collections::BTreeMap;

use gent_core::{
    AgentChatCompactionEffect, AgentChatCompactionState, reduce_agent_chat_compaction,
};
use gent_ports::{AgentChatReadLedger, IngressMode, Ledger};
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

/// A durably retained fact, rejected reducer fact, or immutable recovery child.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCompactionResult {
    DeniedObserver,
    Recorded(AgentChatCompactionEffect),
    Recovered(gent_types::AgentChatSelectionSwitched),
}

/// Unadvertised daemon composition over the pure reducer and atomic child-switch ledger.
///
/// A provider adapter must have already discarded its raw frame before constructing the request.
/// The ingress never receives a provider-native session value and is not reachable from transport.
#[derive(Debug)]
pub(crate) struct PrivateCompactionIngress<L> {
    ledger: L,
    recovery: AgentChatCompactionRecoveryService<L>,
    authority: AgentChatCompactionRecoveryAuthority,
    states: BTreeMap<String, AgentChatCompactionState>,
}

impl<L: Clone> PrivateCompactionIngress<L> {
    #[must_use]
    pub(crate) fn new(ledger: L, authority: AgentChatCompactionRecoveryAuthority) -> Self {
        Self {
            recovery: AgentChatCompactionRecoveryService::new(ledger.clone(), authority),
            ledger,
            authority,
            states: BTreeMap::new(),
        }
    }
}

impl<L: Clone + Ledger + gent_ports::AgentChatSelectionLedger + AgentChatReadLedger>
    PrivateCompactionIngress<L>
{
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
        let state = self
            .states
            .get(&request.run_id.0)
            .cloned()
            .unwrap_or_default();
        let (next, effect) = reduce_agent_chat_compaction(state, source.cursor, &request.fact);
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
        self.states.insert(request.run_id.0, next);
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

fn invariant(message: &str) -> RuntimeError {
    RuntimeError::Ledger(gent_ports::LedgerError::Invariant(message.into()))
}
