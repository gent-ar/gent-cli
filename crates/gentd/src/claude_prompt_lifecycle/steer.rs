use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{AgentChatRunId, DurableTurnPhase, HostEpoch};

use super::{ClaudePromptExecution, ClaudePromptLifecycle, missing_binding, start::provider_input};

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
    pub(super) fn steer_active(&mut self, host_epoch: HostEpoch) -> Result<(), RuntimeError> {
        let run_ids = self
            .active
            .iter()
            .filter(|(run_id, binding)| {
                !binding.settled
                    && !binding.interrupt_requested
                    && self.runner.has_claude_session(run_id)
            })
            .map(|(run_id, _)| run_id.clone())
            .collect::<Vec<_>>();
        let ledger = self.runtime.ledger();
        for run_id in run_ids {
            while let Some((prompt, _)) = ledger.claim_steered_agent_chat_prompt(
                &self.coordinator_id,
                host_epoch,
                &AgentChatRunId(run_id.clone()),
            )? {
                let message_id = prompt.message.message_id.clone();
                let delivered =
                    provider_input(&self.runtime, &prompt.message).and_then(|(text, content)| {
                        self.runner
                            .steer_claude_prompt(&run_id, &message_id, &text, &content)
                            .map_err(RuntimeError::from)
                    });
                if let Err(error) = delivered {
                    eprintln!("Claude steer for prompt {message_id} was not delivered: {error}");
                    self.runtime.release_unstarted_prompt_launch(
                        &message_id,
                        &self.coordinator_id,
                        host_epoch,
                    )?;
                    break;
                }
                self.active
                    .get_mut(&run_id)
                    .ok_or_else(missing_binding)?
                    .steers
                    .push(prompt);
            }
        }
        Ok(())
    }

    pub(super) fn consume_steer(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        message_id: &str,
    ) -> Result<(), RuntimeError> {
        let binding = self.active.get(run_id).ok_or_else(missing_binding)?;
        let Some(index) = binding
            .steers
            .iter()
            .position(|steer| steer.message.message_id == message_id)
        else {
            return Ok(());
        };
        if binding.settled {
            return self.start_steer_turn(run_id, host_epoch, index);
        }
        let turn_id = binding.prompt.message.turn_id.clone();
        self.runtime.ledger().deliver_steered_agent_chat_prompt(
            message_id,
            &self.coordinator_id,
            host_epoch,
            &turn_id,
        )?;
        self.active
            .get_mut(run_id)
            .ok_or_else(missing_binding)?
            .steers
            .remove(index);
        Ok(())
    }

    pub(super) fn settle_turn(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        phase: DurableTurnPhase,
    ) -> Result<(), RuntimeError> {
        self.settle_if_open(run_id, host_epoch, phase)?;
        if self
            .active
            .get(run_id)
            .is_some_and(|binding| binding.interrupt_requested || binding.steers.is_empty())
        {
            return Ok(());
        }
        self.start_steer_turn(run_id, host_epoch, 0)
    }

    fn start_steer_turn(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        index: usize,
    ) -> Result<(), RuntimeError> {
        let message_id = self.active.get(run_id).ok_or_else(missing_binding)?.steers[index]
            .message
            .message_id
            .clone();
        self.runtime.ledger().start_steered_agent_chat_prompt_turn(
            &message_id,
            &self.coordinator_id,
            host_epoch,
        )?;
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        binding.prompt = binding.steers.remove(index);
        binding.settled = false;
        Ok(())
    }

    pub(super) fn release_steers(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        let Some(binding) = self.active.get_mut(run_id) else {
            return Ok(());
        };
        let steers = std::mem::take(&mut binding.steers);
        for steer in steers {
            self.runtime.release_unstarted_prompt_launch(
                &steer.message.message_id,
                &self.coordinator_id,
                host_epoch,
            )?;
        }
        Ok(())
    }
}
