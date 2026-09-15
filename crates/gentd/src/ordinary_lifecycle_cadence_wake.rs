use std::sync::Arc;

use gent_ports::AgentChatReadLedger;

use gent_protocol::{PromptProviderProvisionFrame, PromptProviderProvisionState};
use gent_types::{AgentChatPromptDisposition, AgentChatProvider};

use crate::prompt_provider_provision_boundary::PromptProviderProvisionPort;

use super::{
    OrdinaryPromptIngress, OrdinaryPromptWake, PromptCommitWake, PromptWake,
    StandalonePromptRelease,
};

impl<L: AgentChatReadLedger> OrdinaryPromptIngress<L> {
    pub(crate) const fn steers_by_interrupt(&self, provider: AgentChatProvider) -> bool {
        matches!(provider, AgentChatProvider::Claurst)
    }

    pub(crate) fn steer_run(
        &self,
        provider: AgentChatProvider,
        prompt: PromptWake,
    ) -> Result<(), String> {
        if self.steers_by_interrupt(provider) {
            return self.interrupt_run(provider, &prompt.run_id.0);
        }
        OrdinaryPromptWake {
            router: Arc::clone(&self.wake.router),
            notify: Arc::clone(&self.wake.notify),
        }
        .wake_after_prompt_commit(prompt)
    }
}

#[derive(Clone)]
pub(crate) struct ProvisionedPromptWake<L> {
    provision: Arc<dyn PromptProviderProvisionPort>,
    ingress: OrdinaryPromptIngress<L>,
}

impl<L> ProvisionedPromptWake<L> {
    pub(crate) fn new(
        provision: Arc<dyn PromptProviderProvisionPort>,
        ingress: OrdinaryPromptIngress<L>,
    ) -> Self {
        Self { provision, ingress }
    }
}

impl<L: AgentChatReadLedger + Send + Sync + 'static> PromptProviderProvisionPort
    for ProvisionedPromptWake<L>
{
    fn confirm(
        &self,
        frame: PromptProviderProvisionFrame,
    ) -> Result<PromptProviderProvisionFrame, String> {
        let reply = self.provision.confirm(frame)?;
        if let PromptProviderProvisionFrame::Result {
            prompt_receipt_id,
            conversation_id,
            run_id,
            state: PromptProviderProvisionState::Completed,
            ..
        } = &reply
        {
            let prompt = PromptWake {
                conversation_id: conversation_id.clone(),
                run_id: run_id.clone(),
                receipt_id: prompt_receipt_id.clone(),
                disposition: AgentChatPromptDisposition::Send,
            };
            match &self.ingress.standalone_readiness {
                Some(readiness) => self.ingress.wake.schedule(prompt, Arc::clone(readiness)),
                None => OrdinaryPromptWake {
                    router: Arc::clone(&self.ingress.wake.router),
                    notify: Arc::clone(&self.ingress.wake.notify),
                }
                .wake_after_prompt_commit(prompt)?,
            }
        }
        Ok(reply)
    }
}

impl<L: AgentChatReadLedger + 'static> OrdinaryPromptWake<L> {
    pub(super) fn schedule(&self, prompt: PromptWake, readiness: Arc<dyn StandalonePromptRelease>) {
        let router = Arc::clone(&self.router);
        let notify = Arc::clone(&self.notify);
        std::thread::spawn(move || {
            let result = router
                .lock()
                .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())
                .and_then(|mut router| {
                    router
                        .wake_after_prompt_commit(prompt.clone())
                        .map_err(|_| "Gent could not prepare the selected provider.".to_owned())
                });
            match result {
                Ok(()) => notify.notify_one(),
                Err(reason) => {
                    let _ = readiness.fail(&prompt, &reason);
                }
            }
        });
    }
}

impl<L: AgentChatReadLedger> PromptCommitWake for OrdinaryPromptWake<L> {
    type Error = String;

    fn wake_after_prompt_commit(&mut self, prompt: PromptWake) -> Result<(), Self::Error> {
        self.router
            .lock()
            .map_err(|_| "ordinary lifecycle router is unavailable".to_owned())?
            .wake_after_prompt_commit(prompt)
            .map_err(|_| "ordinary lifecycle wake was rejected".to_owned())?;
        self.notify.notify_one();
        Ok(())
    }
}
