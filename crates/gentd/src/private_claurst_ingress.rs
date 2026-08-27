use std::collections::BTreeMap;

use gent_ports::{
    ClaurstDrainRequest, ClaurstSessionBinding, ClaurstSourceId, ClaurstStartRequest,
    ClaurstSubmitRequest, GoalLedger, Ledger, MAX_PRIVATE_CLAURST_DRAIN_FACTS,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PrivateClaurstBridge,
    RunCheckpointLedger, RunLifecycleFactLedger, TranscriptLedger,
};
use gent_runtime::{
    Coordinator, ProviderLifecycleEffect, ProviderLifecycleIngress, ProviderRunAuthority,
    RuntimeError,
};
use gent_types::{AgentChatConversationId, DurableTurnPhase, HostEpoch};

mod goal;
mod validation;
use validation::{event_id, invariant, restored, terminal_name, validate_batch, validate_binding};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PrivateClaurstDrain {
    pub facts: u16,
    pub terminal: bool,
    pub terminal_phase: Option<DurableTurnPhase>,
}

#[derive(Clone, Debug)]
struct BoundSource {
    binding: ClaurstSessionBinding,
    conversation_id: Option<AgentChatConversationId>,
    turn_id: Option<String>,
    after_cursor: u64,
    terminal: bool,
    cancellation_requested: bool,
}

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
    L: Clone
        + std::fmt::Debug
        + Ledger
        + GoalLedger
        + RunCheckpointLedger
        + RunLifecycleFactLedger
        + NormalizedSessionBatchLedger
        + TranscriptLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::AgentChatWorkspaceLedger,
    B: PrivateClaurstBridge,
{
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

    pub(crate) async fn start(
        &mut self,
        mut request: ClaurstStartRequest,
        host_epoch: HostEpoch,
    ) -> Result<ClaurstSessionBinding, RuntimeError> {
        if request.goal.is_some() {
            return Err(invariant("private Claurst goal must be resolved by Gent"));
        }
        request
            .validate()
            .map_err(|_| invariant("private Claurst start input is invalid"))?;
        if self.sources.contains_key(&request.source_id) {
            return Err(invariant("private Claurst source is already bound"));
        }
        request.goal = goal::resolve(
            &self.ledger,
            &request.context.conversation_id,
            &request.run_id,
            &request.source_id,
        )?;
        let binding = self.bridge.start(request.clone()).await?;
        if binding.run_id != request.run_id || binding.source_id != request.source_id {
            return Err(invariant("private Claurst start returned another source"));
        }
        self.bind_with_conversation(
            binding.clone(),
            host_epoch,
            Some(request.context.conversation_id),
            Some(request.turn_id),
        )
        .await?;
        Ok(binding)
    }

    pub(crate) async fn bind(
        &mut self,
        binding: ClaurstSessionBinding,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.bind_with_conversation(binding, host_epoch, None, None)
            .await
    }

    async fn bind_with_conversation(
        &mut self,
        binding: ClaurstSessionBinding,
        host_epoch: HostEpoch,
        conversation_id: Option<AgentChatConversationId>,
        turn_id: Option<String>,
    ) -> Result<(), RuntimeError> {
        validate_binding(&binding)?;
        if self.sources.contains_key(&binding.source_id) {
            return Err(invariant("private Claurst source is already bound"));
        }
        self.lifecycle.record(
            event_id(&binding.source_id, "session"),
            &binding.run_id,
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
                conversation_id,
                turn_id,
                after_cursor,
                terminal,
                cancellation_requested: false,
            },
        );
        Ok(())
    }

    pub(crate) async fn submit(
        &self,
        mut request: ClaurstSubmitRequest,
    ) -> Result<(), RuntimeError> {
        if request.goal.is_some() {
            return Err(invariant("private Claurst goal must be resolved by Gent"));
        }
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
        if let Some(conversation_id) = state.conversation_id.as_ref() {
            request.goal = goal::resolve(
                &self.ledger,
                conversation_id,
                &state.binding.run_id,
                &state.binding.source_id,
            )?;
        }
        self.bridge.submit(request).await?;
        Ok(())
    }

    pub(crate) async fn cancel_run(&mut self, run_id: &str) -> Result<(), RuntimeError> {
        let state = self
            .sources
            .values_mut()
            .find(|state| state.binding.run_id == run_id && !state.terminal)
            .ok_or_else(|| invariant("private Claurst run is not active"))?;
        if state.cancellation_requested {
            return Err(invariant(
                "private Claurst run cancellation is already requested",
            ));
        }
        self.bridge.cancel(state.binding.clone()).await?;
        state.cancellation_requested = true;
        Ok(())
    }

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
        for permission in &batch.permissions {
            self.record_permission_request(&state, permission, host_epoch)
                .await?;
        }
        for fact in &batch.facts {
            self.record_fact(&state, fact, host_epoch)?;
        }
        let terminal_phase = batch
            .terminal
            .as_ref()
            .map(|terminal| terminal_phase(*terminal));
        let terminal = terminal_phase.is_some();
        let terminal_kind = batch.terminal.map(terminal_name);
        if let Some(kind) = terminal_kind {
            self.lifecycle.record(
                event_id(source_id, kind),
                &state.binding.run_id,
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
            terminal_phase,
        })
    }
}

const fn terminal_phase(terminal: gent_ports::ClaurstTerminal) -> DurableTurnPhase {
    match terminal {
        gent_ports::ClaurstTerminal::Completed => DurableTurnPhase::Completed,
        gent_ports::ClaurstTerminal::Interrupted => DurableTurnPhase::Interrupted,
        gent_ports::ClaurstTerminal::Failed { .. } => DurableTurnPhase::Failed,
    }
}

#[path = "private_claurst_ingress_checkpoint.rs"]
mod checkpoint;
#[path = "private_claurst_ingress_permission.rs"]
mod permission;
#[path = "private_claurst_ingress_projection.rs"]
mod projection;
