use std::collections::BTreeMap;

use async_trait::async_trait;
use gent_ports::{
    AgentChatConversationConfigLedger, AgentChatPromptDispatchLedger, AgentChatReadLedger,
    AgentChatRunContextReader, AgentChatWorkspaceLedger, AttachmentLedger, ContextCompactionLedger,
    ConversationContentReader, GoalLedger, Ledger, NormalizedSessionBatchLedger,
    PendingPermissionLedger, PolicyLedger, PrivateClaurstBridge, RunCheckpointLedger,
    RunLifecycleFactLedger, TranscriptLedger,
};
use gent_runtime::{
    AgentChatPromptDispatchAuthority, AgentChatPromptDispatchService, AgentChatRunContextService,
    ConversationContextArtifactService, Coordinator,
};
use gent_types::{
    AgentChatConversationId, AgentChatPromptSaved, HostEpoch, NormalizedTranscriptAppend,
    NormalizedTranscriptKind,
};

use crate::{
    claurst_runtime_factory::{ClaurstRuntimeFactory, ReadyClaurstRuntime},
    ordinary_lifecycle_cadence::AsyncOrdinaryLifecycleHost,
    private_claurst_ingress::PrivateClaurstIngress,
};

mod compaction;
mod drain;
mod start;

pub(crate) trait ClaurstLifecycleLedger:
    Clone
    + std::fmt::Debug
    + Ledger
    + AgentChatPromptDispatchLedger
    + AgentChatRunContextReader
    + ConversationContentReader
    + ContextCompactionLedger
    + TranscriptLedger
    + RunCheckpointLedger
    + RunLifecycleFactLedger
    + NormalizedSessionBatchLedger
    + GoalLedger
    + PendingPermissionLedger
    + PolicyLedger
    + AttachmentLedger
    + AgentChatWorkspaceLedger
    + AgentChatConversationConfigLedger
    + AgentChatReadLedger
{
}

impl<T> ClaurstLifecycleLedger for T where
    T: Clone
        + std::fmt::Debug
        + Ledger
        + AgentChatPromptDispatchLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + ContextCompactionLedger
        + TranscriptLedger
        + RunCheckpointLedger
        + RunLifecycleFactLedger
        + NormalizedSessionBatchLedger
        + GoalLedger
        + PendingPermissionLedger
        + PolicyLedger
        + AttachmentLedger
        + AgentChatWorkspaceLedger
        + AgentChatConversationConfigLedger
        + AgentChatReadLedger
{
}

#[derive(Debug)]
struct ActivePrompt {
    saved: AgentChatPromptSaved,
    work: compaction::Work,
}

#[derive(Debug)]
pub(crate) struct ClaurstPromptLifecycle<L, B, F = ReadyClaurstRuntime> {
    ledger: L,
    dispatches: AgentChatPromptDispatchService<L>,
    contexts: AgentChatRunContextService<L>,
    artifacts: ConversationContextArtifactService<L>,
    lease_coordinator: Coordinator<L>,
    ingress: PrivateClaurstIngress<L, B>,
    runtime: F,
    coordinator_id: String,
    host_epoch: HostEpoch,
    active: BTreeMap<gent_ports::ClaurstSourceId, ActivePrompt>,
    recovery_complete: bool,
    shutdown_requested: bool,
}

impl<L, B> ClaurstPromptLifecycle<L, B, ReadyClaurstRuntime>
where
    L: ClaurstLifecycleLedger,
    B: PrivateClaurstBridge,
{
    #[must_use]
    pub(crate) fn new(ledger: L, bridge: B, coordinator_id: String, host_epoch: HostEpoch) -> Self {
        Self::new_with_runtime(
            ledger,
            bridge,
            ReadyClaurstRuntime,
            coordinator_id,
            host_epoch,
        )
    }
}

impl<L, B, F> ClaurstPromptLifecycle<L, B, F>
where
    L: ClaurstLifecycleLedger,
    B: PrivateClaurstBridge,
    F: ClaurstRuntimeFactory,
{
    #[must_use]
    pub(crate) fn new_with_runtime(
        ledger: L,
        bridge: B,
        runtime: F,
        coordinator_id: String,
        host_epoch: HostEpoch,
    ) -> Self {
        Self {
            ledger: ledger.clone(),
            dispatches: AgentChatPromptDispatchService::new(
                ledger.clone(),
                AgentChatPromptDispatchAuthority::Approved,
            ),
            contexts: AgentChatRunContextService::new(ledger.clone()),
            artifacts: ConversationContextArtifactService::new(ledger.clone()),
            lease_coordinator: Coordinator::new(
                ledger.clone(),
                gent_types::CapabilitySet::default(),
            ),
            ingress: PrivateClaurstIngress::new(
                Coordinator::new(ledger.clone(), gent_types::CapabilitySet::default()),
                ledger,
                bridge,
                coordinator_id.clone(),
            ),
            runtime,
            coordinator_id,
            host_epoch,
            active: BTreeMap::new(),
            recovery_complete: false,
            shutdown_requested: false,
        }
    }

    async fn stop_failed_runtime(&self, saved: &AgentChatPromptSaved, error: String) -> String {
        match self
            .runtime
            .after_prompt_failed(&saved.message.conversation_id)
            .await
        {
            Ok(()) => error,
            Err(cleanup) => format!("{error}; Claurst runtime cleanup failed: {cleanup}"),
        }
    }

    fn record_notice(
        &self,
        saved: &AgentChatPromptSaved,
        event: &str,
        text: String,
    ) -> Result<(), String> {
        self.ledger
            .append_normalized_transcript(
                &AgentChatConversationId(saved.message.conversation_id.clone()),
                &NormalizedTranscriptAppend {
                    event_id: format!("{event}:{}", saved.message.message_id),
                    turn_id: saved.message.turn_id.clone(),
                    run_id: saved.run_id.0.clone(),
                    kind: NormalizedTranscriptKind::Notice,
                    text,
                    is_partial: false,
                },
            )
            .map(|_| ())
            .map_err(display)
    }

    pub(crate) async fn respond_permission(
        &mut self,
        response: gent_types::PermissionDecisionResponse,
    ) -> Result<(), String> {
        self.ingress
            .respond_permission(response)
            .await
            .map_err(display)
    }

    pub(crate) async fn interrupt_run(&mut self, run_id: &str) -> Result<(), String> {
        if self.interrupt_compaction(run_id) {
            return Ok(());
        }
        self.ingress.cancel_run(run_id).await.map_err(display)
    }

    pub(crate) async fn respond_permission_with_receipt(
        &mut self,
        response: gent_types::PermissionDecisionResponse,
        receipt_id: gent_types::ReceiptId,
    ) -> Result<gent_types::Receipt, String> {
        self.ingress
            .respond_permission_with_receipt(response, receipt_id)
            .await
            .map_err(display)
    }
}

#[async_trait]
impl<L, B, F> AsyncOrdinaryLifecycleHost for ClaurstPromptLifecycle<L, B, F>
where
    L: ClaurstLifecycleLedger + Send,
    B: PrivateClaurstBridge + Send + std::fmt::Debug,
    F: ClaurstRuntimeFactory,
{
    async fn respond_claurst_permission_with_receipt(
        &mut self,
        response: gent_types::PermissionDecisionResponse,
        receipt_id: gent_types::ReceiptId,
    ) -> Result<gent_types::Receipt, String> {
        self.respond_permission_with_receipt(response, receipt_id)
            .await
    }

    async fn interrupt_claurst_run(&mut self, run_id: &str) -> Result<(), String> {
        self.interrupt_run(run_id).await
    }
    async fn activate_recovery(&mut self) -> Result<(), String> {
        self.dispatches.recover(self.host_epoch).map_err(display)?;
        self.recovery_complete = true;
        Ok(())
    }

    async fn drive_once(&mut self) -> Result<bool, String> {
        if !self.recovery_complete {
            return Err("Claurst lifecycle was driven before durable recovery".into());
        }
        let draining = self.drain_active().await?;
        if self.shutdown_requested {
            return Ok(draining);
        }
        let started = if draining {
            false
        } else {
            self.claim_and_start().await?
        };
        Ok(draining || started || !self.active.is_empty())
    }

    async fn begin_shutdown_after_recovery(&mut self) -> Result<(), String> {
        self.shutdown_requested = true;
        Ok(())
    }

    async fn respond_claurst_permission(
        &mut self,
        response: gent_types::PermissionDecisionResponse,
    ) -> Result<(), String> {
        self.respond_permission(response).await
    }

    fn shutdown_complete(&self) -> bool {
        self.shutdown_requested && self.active.is_empty()
    }
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
#[path = "claurst_prompt_lifecycle_compaction_tests.rs"]
mod compaction_tests;
#[cfg(test)]
#[path = "claurst_prompt_lifecycle_tests.rs"]
mod tests;
