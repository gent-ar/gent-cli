use std::sync::Arc;

use gent_ports::AgentChatPromptDispatchLedger;
use gent_protocol::LocalModelDownloadFailure;
use gent_runtime::AgentChatReadService;
use gent_store::SqliteLedger;
use gent_types::{AgentChatProvider, HostEpoch, PromptHoldReason, ProviderPromptReadinessBinding};
use tokio::sync::Notify;

use super::PromptWake;
use crate::local_model_jobs::{DownloadHolder, DownloadProgress, DownloadStart};

#[derive(Clone)]
pub(super) struct StandaloneReadiness {
    reads: AgentChatReadService<SqliteLedger>,
    dispatches: SqliteLedger,
    host_epoch: HostEpoch,
    public_readiness: crate::standalone_provider_readiness::StandalonePublicProviderReadinessPort,
    models: Option<crate::standalone_authority_composition::StandaloneClaurstModels>,
    waiters: waiters::AdmissionWaiters,
}

impl StandaloneReadiness {
    pub(super) fn new(
        ledger: SqliteLedger,
        host_epoch: HostEpoch,
        models: Option<crate::standalone_authority_composition::StandaloneClaurstModels>,
    ) -> Self {
        Self::new_with_public_readiness(
            ledger,
            host_epoch,
            models,
            Arc::new(crate::standalone_provider_readiness::AllowStandalonePublicProviders),
        )
    }

    pub(super) fn new_with_public_readiness(
        ledger: SqliteLedger,
        host_epoch: HostEpoch,
        models: Option<crate::standalone_authority_composition::StandaloneClaurstModels>,
        public_readiness: crate::standalone_provider_readiness::StandalonePublicProviderReadinessPort,
    ) -> Self {
        Self {
            reads: AgentChatReadService::new(ledger.clone()),
            dispatches: ledger,
            host_epoch,
            public_readiness,
            models,
            waiters: waiters::AdmissionWaiters::default(),
        }
    }
}

impl std::fmt::Debug for StandaloneReadiness {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("StandaloneReadiness(..)")
    }
}

pub(super) trait StandalonePromptRelease: Send + Sync + std::fmt::Debug {
    fn provider(&self, prompt: &PromptWake) -> Result<AgentChatProvider, String>;
    fn provision_claurst(
        &self,
        prompt: PromptWake,
        notify: std::sync::Arc<Notify>,
    ) -> Result<(), String>;
    fn release(&self, prompt: &PromptWake) -> Result<StandalonePromptReleaseOutcome, String>;
    fn fail(&self, prompt: &PromptWake, reason: &str) -> Result<(), String>;
    fn cancel_held_prompts(&self, _: &str) -> Result<bool, String> {
        Ok(false)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StandalonePromptReleaseOutcome {
    Routed,
    Claurst,
    Held,
}

impl StandalonePromptRelease for StandaloneReadiness {
    fn provider(&self, prompt: &PromptWake) -> Result<AgentChatProvider, String> {
        self.reads
            .run_selection(&prompt.conversation_id.0, &prompt.run_id.0)
            .map(|selection| selection.provider)
            .map_err(|error| error.to_string())
    }

    fn provision_claurst(
        &self,
        prompt: PromptWake,
        notify: std::sync::Arc<Notify>,
    ) -> Result<(), String> {
        let model_id = self
            .reads
            .run_selection(&prompt.conversation_id.0, &prompt.run_id.0)
            .map_err(|error| error.to_string())?
            .model;
        let models = self
            .models
            .clone()
            .ok_or_else(|| "Claurst local model authority is unavailable".to_owned())?;
        self.waiters.wait(&prompt, &waiters::model(&model_id))?;
        let readiness = self.clone();
        tokio::spawn(async move {
            let holder = DownloadHolder::Prompt(prompt.receipt_id.0.clone());
            let started = {
                let models = models.clone();
                let model_id = model_id.clone();
                let holder = holder.clone();
                tokio::task::spawn_blocking(move || models.downloads.start(&model_id, holder)).await
            };
            let mut progress = match started {
                Ok(Ok(DownloadStart::Downloading(progress))) => progress,
                Ok(Ok(DownloadStart::Ready { .. })) => {
                    if readiness.release_waiting(&prompt) {
                        notify.notify_one();
                    }
                    return;
                }
                Ok(Err(reason)) => {
                    let _ = readiness.fail_download(&prompt, reason);
                    return;
                }
                Err(_) => {
                    let _ = readiness
                        .fail_download(&prompt, LocalModelDownloadFailure::TransportFailed);
                    return;
                }
            };
            if !readiness.waiters.is_waiting(&prompt) {
                models.downloads.release(&model_id, &holder);
                return;
            }
            if let Err(error) = readiness.hold(&prompt, PromptHoldReason::ModelDownload) {
                models.downloads.release(&model_id, &holder);
                let reason = crate::local_model_events::failure_for(&error);
                let _ = readiness.fail_download(&prompt, reason);
                return;
            }
            let mut current = *progress.borrow_and_update();
            while let DownloadProgress::Downloading { .. } = current {
                if progress.changed().await.is_err() {
                    current = DownloadProgress::Failed(LocalModelDownloadFailure::TransportFailed);
                    break;
                }
                current = *progress.borrow_and_update();
            }
            match current {
                DownloadProgress::Complete { .. } => {
                    if readiness.release_waiting(&prompt) {
                        notify.notify_one();
                    }
                }
                DownloadProgress::Failed(reason) => {
                    let _ = readiness.fail_download(&prompt, reason);
                }
                DownloadProgress::Downloading { .. } => {}
            }
        });
        Ok(())
    }

    fn cancel_held_prompts(&self, run_id: &str) -> Result<bool, String> {
        self.cancel_held(run_id)
    }

    fn release(&self, prompt: &PromptWake) -> Result<StandalonePromptReleaseOutcome, String> {
        let selection = self
            .reads
            .run_selection(&prompt.conversation_id.0, &prompt.run_id.0)
            .map_err(|error| error.to_string())?;
        if !matches!(
            selection.provider,
            AgentChatProvider::Claude | AgentChatProvider::Codex | AgentChatProvider::Claurst
        ) {
            return Err("standalone authority does not enable the selected provider".into());
        }
        if matches!(
            selection.provider,
            AgentChatProvider::Claude | AgentChatProvider::Codex
        ) && !self.public_readiness.is_ready(selection.provider)?
        {
            self.hold(prompt, PromptHoldReason::ProviderInstall)?;
            self.waiters
                .wait(prompt, &waiters::provider(selection.provider))?;
            return Ok(StandalonePromptReleaseOutcome::Held);
        }
        let binding = ProviderPromptReadinessBinding {
            prompt_receipt_id: prompt.receipt_id.clone(),
            conversation_id: prompt.conversation_id.clone(),
            run_id: prompt.run_id.clone(),
            provider: selection.provider,
        };
        let (command, terminal) =
            crate::prompt_readiness_admission::decision(&binding, self.host_epoch)?;
        self.dispatches
            .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .map_err(|error| error.to_string())
            .map(|_| match selection.provider {
                AgentChatProvider::Claurst => StandalonePromptReleaseOutcome::Claurst,
                AgentChatProvider::Claude | AgentChatProvider::Codex => {
                    StandalonePromptReleaseOutcome::Routed
                }
            })
    }

    fn fail(&self, prompt: &PromptWake, reason: &str) -> Result<(), String> {
        self.settle_held_prompt(prompt, gent_types::PromptAdmissionExit::Failed, reason)
    }
}

#[path = "ordinary_lifecycle_cadence_standalone_settlement.rs"]
mod settlement;
#[path = "ordinary_lifecycle_cadence_standalone_waiters.rs"]
mod waiters;

#[cfg(test)]
#[path = "ordinary_lifecycle_cadence_standalone_tests.rs"]
mod tests;
