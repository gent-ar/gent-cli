//! Private, credential-free Claurst lifecycle drain, deliberately absent from daemon bootstrap.

use std::collections::BTreeMap;

use gent_ports::{
    ClaurstCheckpoint, ClaurstDrainRequest, ClaurstFactValue, ClaurstNormalizedFact,
    ClaurstSessionBinding, ClaurstSourceId, ClaurstStartRequest, ClaurstSubmitRequest, Ledger,
    MAX_PRIVATE_CLAURST_DRAIN_FACTS, PrivateClaurstBridge, RunCheckpointLedger,
    RunProjectionLedger,
};
use gent_runtime::{
    Coordinator, ProviderLifecycleEffect, ProviderLifecycleIngress, ProviderRunAuthority,
    RuntimeError,
};
use gent_types::{HostEpoch, RunCheckpointRecord};

mod validation;
use validation::{
    checkpoint_id, event_id, invariant, restored, terminal_name, validate_batch, validate_binding,
};

/// Aggregate result returned only after all accepted facts have durable source records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PrivateClaurstDrain {
    pub facts: u16,
    pub terminal: bool,
}

#[derive(Clone, Debug)]
struct BoundSource {
    binding: ClaurstSessionBinding,
    after_cursor: u64,
    terminal: bool,
}

/// Daemon-only lifecycle owner for a bridge source that was reserved elsewhere.
///
/// It starts and follows up only through the typed private bridge and exposes no public
/// transport. Drain results return only after durable lifecycle records and checkpoints exist.
#[derive(Debug)]
pub(crate) struct PrivateClaurstIngress<L, B> {
    coordinator: Coordinator<L>,
    ledger: L,
    lifecycle: ProviderLifecycleIngress<L>,
    bridge: B,
    coordinator_id: String,
    sources: BTreeMap<ClaurstSourceId, BoundSource>,
}

impl<L, B> PrivateClaurstIngress<L, B>
where
    L: Clone + Ledger + RunCheckpointLedger + RunProjectionLedger,
    B: PrivateClaurstBridge,
{
    /// Creates an unadvertised ingress. A separate private composition must explicitly own it.
    #[must_use]
    pub(crate) fn new(
        coordinator: Coordinator<L>,
        ledger: L,
        bridge: B,
        coordinator_id: String,
    ) -> Self {
        Self {
            lifecycle: ProviderLifecycleIngress::new(
                coordinator.clone(),
                ProviderRunAuthority::PrivateClaurstBridge,
            ),
            coordinator,
            ledger,
            bridge,
            coordinator_id,
            sources: BTreeMap::new(),
        }
    }

    /// Starts a fresh private source from daemon-owned normalized input, then persists its bind.
    ///
    /// The private bridge receives no app/client-owned provider configuration. A returned session
    /// must exactly match the requested run and source before its lifecycle is recorded.
    pub(crate) async fn start(
        &mut self,
        request: ClaurstStartRequest,
        host_epoch: HostEpoch,
    ) -> Result<ClaurstSessionBinding, RuntimeError> {
        request
            .validate()
            .map_err(|_| invariant("private Claurst start input is invalid"))?;
        if self.sources.contains_key(&request.source_id) {
            return Err(invariant("private Claurst source is already bound"));
        }
        let binding = self.bridge.start(request.clone()).await?;
        if binding.run_id != request.run_id || binding.source_id != request.source_id {
            return Err(invariant("private Claurst start returned another source"));
        }
        self.bind(binding.clone(), host_epoch).await?;
        Ok(binding)
    }

    /// Persists the opaque session binding before the bridge may be drained.
    pub(crate) async fn bind(
        &mut self,
        binding: ClaurstSessionBinding,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        validate_binding(&binding)?;
        if self.sources.contains_key(&binding.source_id) {
            return Err(invariant("private Claurst source is already bound"));
        }
        self.lifecycle.record(
            event_id(&binding.source_id, "session"),
            binding.run_id.clone(),
            &self.coordinator_id,
            host_epoch,
            ProviderLifecycleEffect::SessionStarted {
                provider_session_id: binding.opaque_session_id.clone(),
            },
        )?;
        self.bridge.bind_session(binding.clone()).await?;
        let (after_cursor, terminal) = restored(&self.coordinator, &binding)?;
        self.sources.insert(
            binding.source_id.clone(),
            BoundSource {
                binding,
                after_cursor,
                terminal,
            },
        );
        Ok(())
    }

    /// Sends one daemon-owned follow-up only to the exact active private session.
    pub(crate) async fn submit(&self, request: ClaurstSubmitRequest) -> Result<(), RuntimeError> {
        request
            .validate()
            .map_err(|_| invariant("private Claurst follow-up input is invalid"))?;
        let state = self
            .sources
            .get(&request.binding.source_id)
            .ok_or_else(|| invariant("private Claurst source is not bound"))?;
        if state.terminal || state.binding != request.binding {
            return Err(invariant(
                "private Claurst follow-up session is unavailable",
            ));
        }
        self.bridge.submit(request).await?;
        Ok(())
    }

    /// Drains one fixed-size batch and persists every normalized source fact before returning.
    pub(crate) async fn drain(
        &mut self,
        source_id: &ClaurstSourceId,
        host_epoch: HostEpoch,
    ) -> Result<PrivateClaurstDrain, RuntimeError> {
        let state = self
            .sources
            .get(source_id)
            .cloned()
            .ok_or_else(|| invariant("private Claurst source is not bound"))?;
        if state.terminal {
            return Err(invariant("private Claurst source is already terminal"));
        }
        let request = ClaurstDrainRequest {
            run_id: state.binding.run_id.clone(),
            source_id: source_id.clone(),
            after_cursor: state.after_cursor,
            limit: MAX_PRIVATE_CLAURST_DRAIN_FACTS,
        };
        let batch = self.bridge.drain(request.clone()).await?;
        let checkpoint = validate_batch(&request, &state.binding, &batch)?;
        for fact in &batch.facts {
            self.record_fact(&state.binding, fact, host_epoch)?;
        }
        let terminal = batch.terminal.is_some();
        let terminal_kind = batch.terminal.map(terminal_name);
        if let Some(kind) = terminal_kind {
            self.lifecycle.record(
                event_id(source_id, kind),
                state.binding.run_id.clone(),
                &self.coordinator_id,
                host_epoch,
                ProviderLifecycleEffect::Terminal {
                    reason: kind.into(),
                },
            )?;
        }
        let after_cursor = checkpoint.cursor;
        if after_cursor != state.after_cursor || terminal {
            self.save_checkpoint(&state.binding, checkpoint, terminal_kind)?;
        }
        let saved = self
            .sources
            .get_mut(source_id)
            .expect("bound source remains present");
        saved.after_cursor = after_cursor;
        saved.terminal = terminal;
        Ok(PrivateClaurstDrain {
            facts: u16::try_from(batch.facts.len()).expect("bridge fact bound fits u16"),
            terminal,
        })
    }

    fn record_fact(
        &self,
        binding: &ClaurstSessionBinding,
        fact: &ClaurstNormalizedFact,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        let effect = match &fact.value {
            ClaurstFactValue::Event(event) => ProviderLifecycleEffect::Normalized(event.clone()),
            ClaurstFactValue::Lifecycle(signal) => {
                ProviderLifecycleEffect::Lifecycle(signal.clone())
            }
        };
        self.lifecycle.record(
            event_id(&binding.source_id, &format!("fact-{}", fact.cursor)),
            binding.run_id.clone(),
            &self.coordinator_id,
            host_epoch,
            effect,
        )?;
        Ok(())
    }

    fn save_checkpoint(
        &self,
        binding: &ClaurstSessionBinding,
        checkpoint: ClaurstCheckpoint,
        terminal_kind: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let records = self.coordinator.run_checkpoints(&binding.run_id)?;
        let sequence = u64::try_from(records.len()).expect("checkpoint count fits u64") + 1;
        let kind =
            terminal_kind.map_or_else(|| format!("fact-{}", checkpoint.cursor), str::to_owned);
        let source_event = self
            .ledger
            .find_event(&event_id(&binding.source_id, &kind))?;
        let event_cursor = source_event.map_or(0, |event| event.cursor);
        if event_cursor == 0 {
            return Err(invariant(
                "private Claurst checkpoint has no durable source event",
            ));
        }
        self.coordinator.save_run_checkpoint(&RunCheckpointRecord {
            checkpoint_id: checkpoint_id(
                &binding.source_id,
                checkpoint.cursor,
                terminal_kind.is_some(),
            ),
            run_id: binding.run_id.clone(),
            sequence,
            event_cursor,
            state_digest_sha256: checkpoint.state_digest_sha256,
        })
    }
}
