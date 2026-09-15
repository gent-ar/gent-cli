use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_protocol::PublicRunOutcome;
use gent_runtime::RuntimeError;
use gent_types::{HostEpoch, NormalizedProviderEvent};

use super::{CodexPromptExecution, CodexPromptLifecycle, start};

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
    pub(super) fn recover_upgraded_session(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<bool, RuntimeError> {
        let Some(binding) = self.active.get_mut(run_id) else {
            return Ok(false);
        };
        if !std::mem::take(&mut binding.upgraded) || binding.settled {
            return Ok(false);
        }
        let prompt = binding.prompt.clone();
        self.record_wire(
            run_id,
            host_epoch,
            &PublicWireFact::Event(NormalizedProviderEvent::TransportDiagnostic {
                classification: gent_types::PROVIDER_SESSION_RECOVERED_DIAGNOSTIC.into(),
            }),
        )?;
        self.runner.release_codex_session(run_id)?;
        self.runtime
            .runs()
            .retire_provider_session(run_id, host_epoch)?;
        let history = self.runtime.contexts.fresh_context_before_message(
            &prompt.message.conversation_id,
            &prompt.message.message_id,
        )?;
        let recreated = super::launch::LaunchSetup::read(&self.runtime, &prompt)?.start(
            &self.runtime,
            &prompt,
            Some(history),
        )?;
        self.runner.prepare_codex_prompt(run_id.into(), recreated)?;
        let started =
            self.runtime
                .runs()
                .start(start::request(run_id, &self.coordinator_id, host_epoch));
        match started.map(|response| response.outcome) {
            Ok(PublicRunOutcome::Started) => Ok(true),
            Ok(_) => {
                self.runner.cancel_codex_prompt(run_id);
                Ok(false)
            }
            Err(error) => {
                self.runner.cancel_codex_prompt(run_id);
                Err(error)
            }
        }
    }
}
