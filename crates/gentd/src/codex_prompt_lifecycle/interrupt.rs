use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;

use super::{CodexPromptExecution, CodexPromptLifecycle, missing_binding};

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
    pub(crate) fn interrupt(&self, run_id: &str) -> Result<(), RuntimeError> {
        if !self.active.contains_key(run_id) {
            return Err(missing_binding());
        }
        self.runner
            .interrupt_codex_turn(run_id)
            .map_err(RuntimeError::from)
    }

    pub(crate) fn interrupt_turn(&self, run_id: &str, turn_id: &str) -> Result<(), RuntimeError> {
        let binding = self.active.get(run_id).ok_or_else(missing_binding)?;
        if !binding.turns.turn_open() || binding.turn_id() != turn_id {
            return Err(missing_binding());
        }
        self.interrupt(run_id)
    }
}
