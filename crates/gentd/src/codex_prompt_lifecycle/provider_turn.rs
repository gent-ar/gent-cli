use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{DurableTurnPhase, HostEpoch};

use super::{CodexPromptExecution, CodexPromptLifecycle, missing_binding};
use crate::provider_run_state::{ProviderSession, ProviderTurnScope};

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
    pub(super) fn begin_provider_turn(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        let ledger = self.runtime.ledger();
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        if binding.turns.turn_open() {
            return Ok(());
        }
        let conversation_id = binding.prompt.message.conversation_id.clone();
        binding.turns.begin_provider_turn(&ProviderTurnScope {
            ledger: &ledger,
            conversation_id: &conversation_id,
            run_id,
            host_epoch,
        })?;
        binding.idle_since = None;
        Ok(())
    }

    pub(super) fn session_reported(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        idle: bool,
    ) -> Result<(), RuntimeError> {
        let ledger = self.runtime.ledger();
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        if binding
            .turns
            .report_session(ProviderSession::from_idle(idle))
        {
            binding
                .turns
                .settle_provider_turn(&ledger, host_epoch, DurableTurnPhase::Completed)?;
        }
        Ok(())
    }
}
