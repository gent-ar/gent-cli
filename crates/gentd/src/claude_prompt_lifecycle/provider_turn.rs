use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{DurableTurnPhase, HostEpoch, NormalizedProviderEvent};

use crate::provider_run_state::{ProviderSession, ProviderTurnScope};

use super::{Binding, ClaudePromptExecution, ClaudePromptLifecycle, missing_binding};

impl Binding {
    pub(super) fn turn_id(&self) -> &str {
        self.turns
            .provider_turn()
            .unwrap_or(&self.prompt.message.turn_id)
    }
}

impl<L, D, R> ClaudePromptLifecycle<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + NormalizedSessionBatchLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(super) fn begin_provider_turn(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        let ledger = self.runtime.ledger();
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        let conversation_id = binding.prompt.message.conversation_id.clone();
        let scope = ProviderTurnScope {
            ledger: &ledger,
            conversation_id: &conversation_id,
            run_id,
            host_epoch,
        };
        let Some(turn_id) = binding.turns.begin_provider_turn(&scope)? else {
            return Ok(());
        };
        binding.idle_since = None;
        self.record_wire(
            run_id,
            host_epoch,
            &PublicWireFact::Event(NormalizedProviderEvent::TurnStarted { turn_id }),
        )?;
        Ok(())
    }

    pub(super) fn session_reported(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        idle: bool,
    ) -> Result<(), RuntimeError> {
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        if binding
            .turns
            .report_session(ProviderSession::from_idle(idle))
        {
            self.settle_continuation(run_id, host_epoch, DurableTurnPhase::Completed)?;
        }
        Ok(())
    }
}
