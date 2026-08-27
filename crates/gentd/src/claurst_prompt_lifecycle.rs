use std::collections::BTreeMap;

use async_trait::async_trait;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationContentReader, Ledger,
    PrivateClaurstBridge, RunCheckpointLedger, RunLease, RunLeaseClaim, RunLifecycleFactLedger,
    TranscriptLedger,
};
use gent_runtime::{
    AgentChatPromptDispatchAuthority, AgentChatPromptDispatchResult,
    AgentChatPromptDispatchService, AgentChatRunContextService, ConversationContextArtifactService,
    ConversationContextRequest, Coordinator,
};
use gent_types::{
    AgentChatPromptSaved, AgentChatProvider, DurableTurnPhase, HostEpoch,
    NormalizedTranscriptAppend, NormalizedTranscriptKind,
};
use sha2::{Digest, Sha256};

use crate::{
    claurst_runtime_factory::{ClaurstRuntimeFactory, ReadyClaurstRuntime},
    ordinary_lifecycle_cadence::AsyncOrdinaryLifecycleHost,
    private_claurst_ingress::PrivateClaurstIngress,
};

#[derive(Debug)]
struct ActivePrompt {
    saved: AgentChatPromptSaved,
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
    L: Clone
        + std::fmt::Debug
        + Ledger
        + AgentChatPromptDispatchLedger
        + gent_ports::AttachmentLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + TranscriptLedger
        + RunCheckpointLedger
        + RunLifecycleFactLedger
        + gent_ports::NormalizedSessionBatchLedger
        + gent_ports::GoalLedger
        + gent_ports::PendingPermissionLedger
        + gent_ports::PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::AgentChatWorkspaceLedger
        + gent_ports::AgentChatConversationConfigLedger,
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
    L: Clone
        + std::fmt::Debug
        + Ledger
        + AgentChatPromptDispatchLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + TranscriptLedger
        + RunCheckpointLedger
        + RunLifecycleFactLedger
        + gent_ports::NormalizedSessionBatchLedger
        + gent_ports::GoalLedger
        + gent_ports::PendingPermissionLedger
        + gent_ports::PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::AgentChatWorkspaceLedger
        + gent_ports::AgentChatConversationConfigLedger,
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

    async fn claim_and_start(&mut self) -> Result<bool, String> {
        match self
            .dispatches
            .claim(
                &self.coordinator_id,
                self.host_epoch,
                AgentChatProvider::Claurst,
            )
            .map_err(display)?
        {
            AgentChatPromptDispatchResult::DeniedObserver
            | AgentChatPromptDispatchResult::Empty => Ok(false),
            AgentChatPromptDispatchResult::Claimed(saved) => {
                let saved = *saved;
                if !self.active.is_empty() {
                    self.dispatches
                        .release_claim(
                            &saved.message.message_id,
                            &self.coordinator_id,
                            self.host_epoch,
                        )
                        .map_err(display)?;
                    return Ok(true);
                }
                let lease = RunLease {
                    run_id: saved.run_id.0.clone(),
                    coordinator_id: self.coordinator_id.clone(),
                    host_epoch: self.host_epoch,
                };
                if matches!(
                    self.lease_coordinator
                        .claim_run_lease(&lease)
                        .map_err(display)?,
                    RunLeaseClaim::Contended(_)
                ) {
                    self.dispatches
                        .release_claim(
                            &saved.message.message_id,
                            &self.coordinator_id,
                            self.host_epoch,
                        )
                        .map_err(display)?;
                    return Err(
                        "standalone Claurst run lease is owned by another coordinator".into(),
                    );
                }
                if let Err(error) = self.runtime.ensure_for_prompt(&saved).await {
                    self.dispatches
                        .fail_prelaunch(
                            &saved.message.message_id,
                            &self.coordinator_id,
                            self.host_epoch,
                            &error,
                        )
                        .map_err(display)?;
                    return Ok(true);
                }
                let attachment_metadata = self
                    .ledger
                    .turn_attachments(&saved.message.turn_id)
                    .map_err(display)?;
                let attachments = match self.runtime.prompt_attachments(&attachment_metadata).await
                {
                    Ok(attachments) => attachments,
                    Err(error) => {
                        self.dispatches
                            .fail_prelaunch(
                                &saved.message.message_id,
                                &self.coordinator_id,
                                self.host_epoch,
                                &error,
                            )
                            .map_err(display)?;
                        return Ok(true);
                    }
                };
                let conversation_config = self
                    .ledger
                    .current_conversation_config(&saved.message.conversation_id)
                    .map_err(display)?;
                let message_text = match conversation_config
                    .as_ref()
                    .filter(|config| config.append_system_prompt)
                    .and_then(|config| config.system_prompt.as_deref())
                {
                    Some(configured) => format!("{configured}\n\n{}", saved.message.text),
                    None => saved.message.text.clone(),
                };
                let prompt = crate::provider_attachments::claurst_prompt_with_files(
                    &message_text,
                    &attachments,
                );
                let attachments = crate::provider_attachments::claurst_images(&attachments);
                self.dispatches
                    .begin_launch(
                        &saved.message.message_id,
                        &self.coordinator_id,
                        self.host_epoch,
                    )
                    .map_err(display)?;
                let source_id = source_id(&saved);
                let context_boundary = self
                    .contexts
                    .resolve(
                        &gent_types::AgentChatConversationId(saved.message.conversation_id.clone()),
                        &saved.run_id,
                    )
                    .map_err(display)?;
                let context = self
                    .artifacts
                    .project(&ConversationContextRequest {
                        conversation_id: gent_types::AgentChatConversationId(
                            saved.message.conversation_id.clone(),
                        ),
                        context_policy: context_boundary.context_policy,
                        context_through_ordinal: context_boundary.context_through_ordinal,
                    })
                    .map_err(display)?;
                let request = gent_ports::ClaurstStartRequest {
                    run_id: saved.run_id.0.clone(),
                    source_id: source_id.clone(),
                    turn_id: saved.message.turn_id.clone(),
                    prompt,
                    context,
                    attachments,
                    goal: None,
                };
                if let Err(error) = self.ingress.start(request, self.host_epoch).await {
                    self.ledger
                        .append_normalized_transcript(
                            &gent_types::AgentChatConversationId(
                                saved.message.conversation_id.clone(),
                            ),
                            &NormalizedTranscriptAppend {
                                event_id: format!(
                                    "claurst-start-failed:{}",
                                    saved.message.message_id
                                ),
                                turn_id: saved.message.turn_id.clone(),
                                run_id: saved.run_id.0.clone(),
                                kind: NormalizedTranscriptKind::Notice,
                                text: format!("Claurst could not start: {error}"),
                                is_partial: false,
                            },
                        )
                        .map_err(display)?;
                    self.dispatches
                        .mark_unprovable(
                            &saved.message.message_id,
                            &self.coordinator_id,
                            self.host_epoch,
                        )
                        .map_err(display)?;
                    return Err(display(error));
                }
                self.dispatches
                    .confirm_started(
                        &saved.message.message_id,
                        &self.coordinator_id,
                        self.host_epoch,
                    )
                    .map_err(display)?;
                self.active.insert(source_id, ActivePrompt { saved });
                Ok(true)
            }
        }
    }

    async fn drain_active(&mut self) -> Result<bool, String> {
        let sources: Vec<_> = self.active.keys().cloned().collect();
        let mut active = false;
        for source_id in sources {
            let drained = match self.ingress.drain(&source_id, self.host_epoch).await {
                Ok(drained) => drained,
                Err(error) => {
                    let saved = self
                        .active
                        .remove(&source_id)
                        .expect("active source remains present while it is drained")
                        .saved;
                    self.ledger
                        .append_normalized_transcript(
                            &gent_types::AgentChatConversationId(
                                saved.message.conversation_id.clone(),
                            ),
                            &NormalizedTranscriptAppend {
                                event_id: format!(
                                    "claurst-drain-failed:{}",
                                    saved.message.message_id
                                ),
                                turn_id: saved.message.turn_id.clone(),
                                run_id: saved.run_id.0.clone(),
                                kind: NormalizedTranscriptKind::Notice,
                                text: format!("Claurst stopped: {error}"),
                                is_partial: false,
                            },
                        )
                        .map_err(display)?;
                    self.dispatches
                        .mark_unprovable(
                            &saved.message.message_id,
                            &self.coordinator_id,
                            self.host_epoch,
                        )
                        .map_err(display)?;
                    return Err(display(error));
                }
            };
            if drained.terminal {
                let saved = self
                    .active
                    .remove(&source_id)
                    .expect("active source remains present")
                    .saved;
                let terminal_phase = drained
                    .terminal_phase
                    .expect("terminal Claurst drain has a terminal phase");
                self.dispatches
                    .settle_terminal(
                        &saved.message.message_id,
                        &self.coordinator_id,
                        self.host_epoch,
                        terminal_phase,
                    )
                    .map_err(display)?;
                if terminal_phase == DurableTurnPhase::Completed {
                    self.runtime
                        .after_prompt_settled(&saved.message.conversation_id)
                        .await?;
                } else {
                    self.runtime
                        .after_prompt_failed(&saved.message.conversation_id)
                        .await?;
                }
            } else {
                active = true;
            }
        }
        Ok(active)
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
    L: Clone
        + std::fmt::Debug
        + Ledger
        + AgentChatPromptDispatchLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + TranscriptLedger
        + RunCheckpointLedger
        + RunLifecycleFactLedger
        + gent_ports::NormalizedSessionBatchLedger
        + gent_ports::GoalLedger
        + gent_ports::PendingPermissionLedger
        + gent_ports::PolicyLedger
        + gent_ports::AgentChatWorkspaceLedger
        + gent_ports::AttachmentLedger
        + gent_ports::AgentChatConversationConfigLedger
        + Send,
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

fn source_id(saved: &AgentChatPromptSaved) -> gent_ports::ClaurstSourceId {
    let material = format!(
        "{}\0{}\0{}",
        saved.run_id.0, saved.message.turn_id, saved.message.message_id
    );
    gent_ports::ClaurstSourceId(format!(
        "gent-{}",
        hex::encode(Sha256::digest(material.as_bytes()))
    ))
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
#[path = "claurst_prompt_lifecycle_tests.rs"]
mod tests;
