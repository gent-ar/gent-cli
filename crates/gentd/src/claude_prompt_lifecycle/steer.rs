use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{AgentChatRunId, DurableTurnPhase, HostEpoch};

use super::{
    ClaudePromptExecution, ClaudePromptLifecycle, launch::provider_input, missing_binding,
};

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
        let ledger = self.runtime.ledger();
        let run_ids = self
            .active
            .iter()
            .filter(|(run_id, binding)| {
                binding.turns.turn_open()
                    && self.runner.has_claude_session(run_id)
                    && ledger
                        .find_run_session_binding(run_id)
                        .is_ok_and(|session| session.is_some())
            })
            .map(|(run_id, _)| run_id.clone())
            .collect::<Vec<_>>();
        for run_id in run_ids {
            let turn_id = self
                .active
                .get(&run_id)
                .ok_or_else(missing_binding)?
                .turn_id()
                .to_owned();
            while let Some(prompt) = ledger.claim_steered_agent_chat_prompt_for_turn(
                &self.coordinator_id,
                host_epoch,
                &AgentChatRunId(run_id.clone()),
                &turn_id,
            )? {
                let message_id = prompt.message.message_id.clone();
                self.runtime
                    .begin_prompt_launch(&message_id, &self.coordinator_id, host_epoch)?;
                let written =
                    provider_input(&self.runtime, &prompt.message).and_then(|(text, content)| {
                        self.runner
                            .steer_claude_prompt(&run_id, &message_id, &text, &content)
                            .map_err(RuntimeError::from)
                    });
                if let Err(error) = written {
                    eprintln!("Claude steer for prompt {message_id} was not delivered: {error}");
                    ledger.reject_steered_agent_chat_prompt_delivery(
                        &message_id,
                        &self.coordinator_id,
                        host_epoch,
                    )?;
                    continue;
                }
                self.active
                    .get_mut(&run_id)
                    .ok_or_else(missing_binding)?
                    .steers
                    .push(prompt);
            }
        }
        self.withdraw_canceled_steers()
    }

    fn withdraw_canceled_steers(&self) -> Result<(), RuntimeError> {
        for (run_id, binding) in &self.active {
            if binding.steers.is_empty() {
                continue;
            }
            for message_id in self
                .runtime
                .ledger()
                .canceled_steered_agent_chat_prompts(&AgentChatRunId(run_id.clone()))?
            {
                if let Err(error) = self.runner.withdraw_claude_prompt(run_id, &message_id) {
                    eprintln!("Claude withdrawal of prompt {message_id} was not written: {error}");
                }
            }
        }
        Ok(())
    }

    pub(super) fn message_started(
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
            if binding.prompt.message.message_id == message_id && binding.turns.prompt_open() {
                return self.settle_continuation(run_id, host_epoch, DurableTurnPhase::Completed);
            }
            return Ok(());
        };
        if !binding.turns.turn_open() {
            return self.start_steer_turn(run_id, host_epoch, index);
        }
        let turn_id = binding.turn_id().to_owned();
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

    pub(super) fn message_cancelled(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        message_id: &str,
    ) -> Result<(), RuntimeError> {
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        let Some(index) = binding
            .steers
            .iter()
            .position(|steer| steer.message.message_id == message_id)
        else {
            if binding.prompt.message.message_id == message_id {
                return self.settle_prompt(run_id, host_epoch, DurableTurnPhase::Interrupted);
            }
            return Ok(());
        };
        binding.steers.remove(index);
        self.return_unconsumed_steer(message_id, host_epoch)
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
        binding.turns.open_prompt();
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
            self.return_unconsumed_steer(&steer.message.message_id, host_epoch)?;
        }
        Ok(())
    }

    fn return_unconsumed_steer(
        &self,
        message_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.runtime
            .release_unstarted_prompt_launch(message_id, &self.coordinator_id, host_epoch)
    }
}
