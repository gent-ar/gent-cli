use gent_ports::{ClaurstSourceId, PrivateClaurstBridge};
use gent_types::AgentChatPromptSaved;

use super::{ClaurstLifecycleLedger, ClaurstPromptLifecycle, display};
use crate::claurst_runtime_factory::ClaurstRuntimeFactory;

impl<L, B, F> ClaurstPromptLifecycle<L, B, F>
where
    L: ClaurstLifecycleLedger,
    B: PrivateClaurstBridge,
    F: ClaurstRuntimeFactory,
{
    pub(super) async fn follow_background(
        &mut self,
        source_id: ClaurstSourceId,
        saved: AgentChatPromptSaved,
    ) {
        if self.ingress.runs_background_commands(&source_id).await {
            let conversation = &saved.message.conversation_id;
            self.background
                .retain(|_, followed| followed.message.conversation_id != *conversation);
            self.background.insert(source_id, saved);
        }
    }

    pub(super) async fn drain_background(&mut self) -> Result<bool, String> {
        let followed: Vec<_> = self.background.keys().cloned().collect();
        let mut progressed = false;
        for source_id in followed {
            if !self.ingress.runs_background_commands(&source_id).await {
                self.background.remove(&source_id);
                continue;
            }
            let conversation = &self.background[&source_id].message.conversation_id;
            if self
                .active
                .values()
                .any(|active| active.saved.message.conversation_id == *conversation)
            {
                continue;
            }
            match self
                .ingress
                .drain_background_commands(&source_id, self.host_epoch)
                .await
            {
                Ok(drained) => progressed |= drained,
                Err(error) => {
                    self.background.remove(&source_id);
                    eprintln!("Claurst background commands stopped reporting: {error}");
                }
            }
        }
        Ok(progressed)
    }

    pub(super) async fn interrupt_background(&mut self, run_id: &str) -> Result<bool, String> {
        let Some(source_id) = self
            .background
            .iter()
            .find(|(_, followed)| followed.run_id.0 == run_id)
            .map(|(source_id, _)| source_id.clone())
        else {
            return Ok(false);
        };
        self.ingress
            .cancel_background_commands(&source_id)
            .await
            .map_err(display)?;
        Ok(true)
    }
}
