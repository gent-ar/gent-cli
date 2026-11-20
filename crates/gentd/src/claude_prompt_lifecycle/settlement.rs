use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{DurableTurnPhase, HostEpoch};

use crate::provider_run_state::TurnSettlement;

use super::{ClaudePromptExecution, ClaudePromptLifecycle, missing_binding};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TurnResult {
    pub(super) message_ids: Vec<String>,
    pub(super) continuation: bool,
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
    pub(super) fn settle_result(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        phase: DurableTurnPhase,
        result: Option<TurnResult>,
    ) -> Result<(), RuntimeError> {
        let Some(result) = result else {
            return self.settle_if_open(run_id, host_epoch, phase);
        };
        if result.continuation {
            self.settle_continuation(run_id, host_epoch, phase)?;
        }
        let binding = self.active.get(run_id).ok_or_else(missing_binding)?;
        if result
            .message_ids
            .contains(&binding.prompt.message.message_id)
        {
            self.settle_prompt(run_id, host_epoch, phase)?;
        }
        Ok(())
    }

    pub(super) fn settle_if_open(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        phase: DurableTurnPhase,
    ) -> Result<(), RuntimeError> {
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        let settled = binding.turns.settle_open(&TurnSettlement {
            runtime: &self.runtime,
            coordinator_id: &self.coordinator_id,
            message_id: &binding.prompt.message.message_id,
            host_epoch,
            phase,
        })?;
        self.summarize_settled(settled, run_id, phase)
    }

    pub(super) fn settle_continuation(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        phase: DurableTurnPhase,
    ) -> Result<(), RuntimeError> {
        let ledger = self.runtime.ledger();
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        if !binding
            .turns
            .settle_provider_turn(&ledger, host_epoch, phase)?
        {
            return Ok(());
        }
        self.summarize_settled(true, run_id, phase)
    }

    pub(super) fn settle_prompt(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        phase: DurableTurnPhase,
    ) -> Result<(), RuntimeError> {
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        let settled = binding.turns.settle_prompt(&TurnSettlement {
            runtime: &self.runtime,
            coordinator_id: &self.coordinator_id,
            message_id: &binding.prompt.message.message_id,
            host_epoch,
            phase,
        })?;
        self.summarize_settled(settled, run_id, phase)
    }

    fn summarize_settled(
        &self,
        settled: bool,
        run_id: &str,
        phase: DurableTurnPhase,
    ) -> Result<(), RuntimeError> {
        let binding = self.active.get(run_id).ok_or_else(missing_binding)?;
        if settled
            && phase == DurableTurnPhase::Completed
            && let Some(hook) = &self.summary_hook
        {
            let _ = hook.schedule(&binding.prompt.message.conversation_id);
        }
        Ok(())
    }
}
