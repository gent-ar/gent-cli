use gent_ports::{AgentChatPromptDispatchLedger, PromptAdmission};
use gent_types::{PromptAdmissionExit, PromptHoldReason, ProviderPromptReadinessFailureBinding};

use super::{PromptWake, StandaloneReadiness};

impl StandaloneReadiness {
    pub(super) fn fail_download(
        &self,
        prompt: &PromptWake,
        reason: gent_protocol::LocalModelDownloadFailure,
    ) -> Result<(), String> {
        if !self.waiters.stop_waiting(prompt) {
            return Ok(());
        }
        let exit = if reason == gent_protocol::LocalModelDownloadFailure::Cancelled {
            PromptAdmissionExit::Cancelled
        } else {
            PromptAdmissionExit::Failed
        };
        self.settle_held_prompt(
            prompt,
            exit,
            crate::local_model_events::failure_text(reason),
        )
    }

    pub(super) fn hold(&self, prompt: &PromptWake, reason: PromptHoldReason) -> Result<(), String> {
        self.dispatches
            .hold_agent_chat_prompt_for_admission(&prompt.receipt_id, self.host_epoch, reason)
            .map_err(|error| error.to_string())
    }

    pub(super) fn fail_prompt(&self, prompt: &PromptWake, reason: &str) -> Result<(), String> {
        if !self.waiters.stop_waiting(prompt) {
            return Ok(());
        }
        self.settle_held_prompt(prompt, PromptAdmissionExit::Failed, reason)
    }

    pub(super) fn release_waiting(&self, prompt: &PromptWake) -> bool {
        if !self.waiters.stop_waiting(prompt) {
            return false;
        }
        let Err(error) = super::StandalonePromptRelease::release(self, prompt) else {
            return true;
        };
        let reason = crate::local_model_events::failure_for(&error);
        let text = crate::local_model_events::failure_text(reason);
        let _ = self.settle_held_prompt(prompt, PromptAdmissionExit::Failed, text);
        false
    }

    pub(super) fn cancel_held(&self, run_id: &str) -> Result<bool, String> {
        let mut cancelled = false;
        for (prompt, resource) in self.waiters.held_for_run(run_id)? {
            match self
                .dispatches
                .agent_chat_prompt_admission(&prompt.receipt_id)
                .map_err(|error| error.to_string())?
            {
                PromptAdmission::Admitted => self.stop_waiting_for(&prompt, &resource),
                PromptAdmission::Installing => {
                    return Err("the held prompt's provider install is already running".into());
                }
                PromptAdmission::Held => {
                    self.settle_held_prompt(
                        &prompt,
                        PromptAdmissionExit::Cancelled,
                        "cancelled by the user",
                    )?;
                    self.stop_waiting_for(&prompt, &resource);
                    cancelled = true;
                }
            }
        }
        Ok(cancelled)
    }

    fn stop_waiting_for(&self, prompt: &PromptWake, resource: &str) {
        self.waiters.stop_waiting(prompt);
        if let (Some(models), Some(model_id)) = (&self.models, resource.strip_prefix("model:")) {
            models.downloads.release(
                model_id,
                &crate::local_model_jobs::DownloadHolder::Prompt(prompt.receipt_id.0.clone()),
            );
        }
    }

    pub(super) fn settle_held_prompt(
        &self,
        prompt: &PromptWake,
        exit: PromptAdmissionExit,
        reason: &str,
    ) -> Result<(), String> {
        let provider = self
            .reads
            .run_selection(&prompt.conversation_id.0, &prompt.run_id.0)
            .map_err(|error| error.to_string())?
            .provider;
        let binding = ProviderPromptReadinessFailureBinding {
            prompt_receipt_id: prompt.receipt_id.clone(),
            conversation_id: prompt.conversation_id.clone(),
            run_id: prompt.run_id.clone(),
            provider,
            exit,
            reason: reason.to_owned(),
        };
        let (command, terminal) =
            crate::prompt_readiness_admission::failure(&binding, self.host_epoch)?;
        self.dispatches
            .fail_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
