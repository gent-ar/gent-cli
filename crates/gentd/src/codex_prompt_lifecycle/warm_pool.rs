use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::AgentChatPromptSaved;

use super::{CodexPromptExecution, CodexPromptLifecycle};
use crate::provider_warm_pool::{WarmRun, evictions};

impl<L, D, R> CodexPromptLifecycle<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + NormalizedSessionBatchLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + gent_ports::AgentChatRunContextReader
        + gent_ports::ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(super) fn make_room_for(
        &mut self,
        prompt: &AgentChatPromptSaved,
    ) -> Result<(), RuntimeError> {
        let evicted = evictions(
            self.active.iter().map(|(run_id, binding)| WarmRun {
                run_id,
                conversation_id: &binding.prompt.message.conversation_id,
                idle: binding.idle(),
                last_dispatch: binding.last_dispatch,
            }),
            &prompt.run_id.0,
            &prompt.message.conversation_id,
            self.warm_capacity,
        );
        for run_id in evicted {
            if self.runner.has_codex_session(&run_id) {
                self.runner.release_codex_session(&run_id)?;
            }
            self.active.remove(&run_id);
        }
        Ok(())
    }

    pub(super) fn mark_dispatched(&mut self, run_id: &str) {
        self.dispatches = self.dispatches.saturating_add(1);
        if let Some(binding) = self.active.get_mut(run_id) {
            binding.last_dispatch = self.dispatches;
        }
    }
}
