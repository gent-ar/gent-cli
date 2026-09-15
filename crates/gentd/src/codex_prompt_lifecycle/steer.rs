use gent_drivers::codex_session::CodexSteerOutcome;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{AgentChatRunId, HostEpoch};

use super::{CodexPromptLifecycle, launch::provider_input, missing_binding};

impl<L, D, R> CodexPromptLifecycle<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + NormalizedSessionBatchLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AttachmentLedger
        + gent_ports::AgentChatReadLedger
        + gent_ports::AgentChatRunContextReader
        + gent_ports::ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: super::CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(crate) fn steer_active(&mut self, host_epoch: HostEpoch) -> Result<(), RuntimeError> {
        let run_ids = self
            .active
            .iter()
            .filter(|(run_id, binding)| {
                !binding.settled && !binding.releasing && self.runner.has_codex_session(run_id)
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
                let delivered = provider_input(&self.runtime, &prompt.message, &run_id).and_then(
                    |(text, attachments)| {
                        self.runner
                            .steer_codex_turn(&run_id, &message_id, &text, &attachments)
                            .map_err(RuntimeError::from)
                    },
                );
                if delivered.is_err() {
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

    pub(super) fn resolve_steer(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        outcome: CodexSteerOutcome,
    ) -> Result<(), RuntimeError> {
        let (message_id, consumed) = match outcome {
            CodexSteerOutcome::Consumed { message_id } => (message_id, true),
            CodexSteerOutcome::Rejected { message_id } => (message_id, false),
        };
        let binding = self.active.get(run_id).ok_or_else(missing_binding)?;
        let Some(index) = binding
            .steers
            .iter()
            .position(|steer| steer.message.message_id == message_id)
        else {
            return Ok(());
        };
        if consumed {
            let turn_id = binding.prompt.message.turn_id.clone();
            self.runtime.ledger().deliver_steered_agent_chat_prompt(
                &message_id,
                &self.coordinator_id,
                host_epoch,
                &turn_id,
            )?;
        } else {
            self.runtime.release_unstarted_prompt_launch(
                &message_id,
                &self.coordinator_id,
                host_epoch,
            )?;
        }
        self.active
            .get_mut(run_id)
            .ok_or_else(missing_binding)?
            .steers
            .remove(index);
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
