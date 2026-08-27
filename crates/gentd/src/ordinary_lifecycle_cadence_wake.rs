use std::sync::Arc;

use gent_ports::AgentChatReadLedger;

use super::{OrdinaryPromptWake, PromptCommitWake, PromptWake, StandalonePromptRelease};

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
