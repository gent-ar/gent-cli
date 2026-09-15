use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_protocol::{PublicRunOutcome, PublicRunResumeRequest};
use gent_runtime::RuntimeError;
use gent_types::{HostEpoch, NormalizedProviderEvent};

use super::{ClaudePromptExecution, ClaudePromptLifecycle, missing_binding, start};

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
    pub(super) fn recover_unavailable_session(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<bool, RuntimeError> {
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        let recoverable = binding.session_recovery == super::SessionRecovery::Unavailable
            && !binding.interrupt_requested
            && !binding.settled;
        if !recoverable {
            return Ok(false);
        }
        binding.session_recovery = super::SessionRecovery::Recovered;
        let prompt = binding.prompt.clone();
        let history = self.runtime.contexts.fresh_context_before_message(
            &prompt.message.conversation_id,
            &prompt.message.message_id,
        )?;
        let mut recreated = start::prompt_start(&self.runtime, &prompt, Some(history))?;
        recreated.recreate_session = true;
        self.record_wire(
            run_id,
            host_epoch,
            &PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic {
                classification: gent_types::PROVIDER_SESSION_RECOVERED_DIAGNOSTIC.into(),
            }),
        )?;
        self.runner
            .prepare_claude_prompt(run_id.into(), recreated)?;
        let resumed = self.runtime.runs().resume(PublicRunResumeRequest {
            run_id: run_id.into(),
            coordinator_id: self.coordinator_id.clone(),
            host_epoch,
        });
        match resumed.map(|response| response.outcome) {
            Ok(PublicRunOutcome::Resumed) => Ok(true),
            Ok(_) => {
                self.runner.cancel_claude_prompt(run_id);
                Ok(false)
            }
            Err(error) => {
                self.runner.cancel_claude_prompt(run_id);
                Err(error)
            }
        }
    }
}
