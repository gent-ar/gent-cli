//! Daemon-owned admission from a held prompt to a verified provider-ready lifecycle wake.

use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatReadLedger, ProvisionedProviderLockReader,
};
use gent_runtime::AgentChatReadService;
use gent_types::{
    AgentChatPromptDisposition, Command, Event, HostEpoch, ProviderPromptReadinessBinding,
    ReceiptId,
};
use sha2::{Digest, Sha256};

use crate::agent_chat_api::{PromptCommitWake, PromptWake};
use crate::private_provider_readiness::{
    PrivateProviderReadiness, PrivateProviderReadinessService,
};

/// Private post-commit authority that releases a held prompt only after daemon-owned readiness.
///
/// It reads the provider from the exact durable run; no client provider, package, receipt, or
/// executable value enters this boundary. Readiness never launches a process. A release commits
/// before the downstream lifecycle is notified, so a failed wake is retry-safe and cannot start
/// a provider inline.
pub(crate) struct ProviderReadyPromptAdmission<R, I, D, W> {
    reads: AgentChatReadService<R>,
    readiness: PrivateProviderReadinessService<I>,
    dispatches: D,
    host_epoch: HostEpoch,
    next: W,
}

impl<R, I, D, W> ProviderReadyPromptAdmission<R, I, D, W> {
    #[must_use]
    pub(crate) fn new(
        reads: AgentChatReadService<R>,
        readiness: PrivateProviderReadinessService<I>,
        dispatches: D,
        host_epoch: HostEpoch,
        next: W,
    ) -> Self {
        Self {
            reads,
            readiness,
            dispatches,
            host_epoch,
            next,
        }
    }
}

impl<R, I, D, W> PromptCommitWake for ProviderReadyPromptAdmission<R, I, D, W>
where
    R: AgentChatReadLedger,
    I: Clone + ProvisionedProviderLockReader,
    D: AgentChatPromptDispatchLedger,
    W: PromptCommitWake,
    W::Error: std::fmt::Display,
{
    type Error = String;

    fn handles_awaiting_readiness(&self) -> bool {
        true
    }

    fn wake_after_prompt_commit(&mut self, prompt: PromptWake) -> Result<(), Self::Error> {
        if prompt.disposition != AgentChatPromptDisposition::Send {
            return Ok(());
        }
        let Some(binding) = self.current_binding(&prompt)? else {
            return Ok(());
        };
        if !matches!(
            self.readiness.assess(binding.provider),
            PrivateProviderReadiness::Ready(_)
        ) {
            return Ok(());
        }
        let (command, terminal) = decision(&binding, self.host_epoch)?;
        self.dispatches
            .release_verified_agent_chat_prompt_after_readiness(&command, &terminal, &binding)
            .map_err(|error| error.to_string())?;
        self.next
            .wake_after_prompt_commit(prompt)
            .map_err(|error| error.to_string())
    }
}

impl<R, I, D, W> ProviderReadyPromptAdmission<R, I, D, W>
where
    R: AgentChatReadLedger,
{
    fn current_binding(
        &self,
        prompt: &PromptWake,
    ) -> Result<Option<ProviderPromptReadinessBinding>, String> {
        let detail = self
            .reads
            .detail(&prompt.conversation_id.0)
            .map_err(|error| error.to_string())?;
        if detail.current_run_id != prompt.run_id.0 {
            return Ok(None);
        }
        let Some(run) = detail
            .runs
            .into_iter()
            .find(|run| run.run_id == prompt.run_id.0)
        else {
            return Err("agent-chat current run is absent from its conversation".into());
        };
        Ok(Some(ProviderPromptReadinessBinding {
            prompt_receipt_id: prompt.receipt_id.clone(),
            conversation_id: prompt.conversation_id.clone(),
            run_id: prompt.run_id.clone(),
            provider: run.selection.provider,
        }))
    }
}

fn decision(
    binding: &ProviderPromptReadinessBinding,
    host_epoch: HostEpoch,
) -> Result<(Command, Event), String> {
    let payload = serde_json::to_value(binding).map_err(|error| error.to_string())?;
    let identity = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(|error| error.to_string())?,
    ));
    let receipt_id = ReceiptId(format!("daemon-readiness:{identity}"));
    let command = Command {
        receipt_id: receipt_id.clone(),
        idempotency_key: format!("daemon-readiness:{identity}"),
        host_epoch,
        kind: "agentChatProviderReadiness".into(),
        payload: payload.clone(),
    };
    let terminal = Event {
        cursor: 0,
        event_id: format!("daemon-readiness:{identity}:ready"),
        receipt_id,
        host_epoch,
        kind: "agentChatProviderReady".into(),
        payload,
    };
    Ok((command, terminal))
}

#[cfg(test)]
#[path = "prompt_readiness_admission_tests.rs"]
mod tests;
