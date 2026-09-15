use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::{AgentChatPromptSaved, DurableTurnPhase, HostEpoch};

use super::{CodexPromptDispatchOutcome, CodexPromptExecution, CodexPromptLifecycle};
use crate::public_driver_runtime::run_failure::{
    failure_fact, is_daemon_fatal, tolerate_run_scoped,
};

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
        + AgentChatRunContextReader
        + ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(super) fn release_outdated_session(&mut self, run_id: &str) -> Result<(), RuntimeError> {
        if self.runner.has_codex_session(run_id)
            && !self.runtime.runs().launched_executable_is_current(run_id)?
        {
            self.runner.release_codex_session(run_id)?;
            self.active.remove(run_id);
        }
        Ok(())
    }

    pub(super) fn fail_run(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        error: RuntimeError,
    ) -> Result<(), RuntimeError> {
        if is_daemon_fatal(&error) {
            return Err(error);
        }
        eprintln!("Codex run {run_id} failed and was stopped: {error}");
        if self.runner.has_codex_session(run_id) {
            let _ = self.runner.release_codex_session(run_id);
        }
        if self
            .active
            .get(run_id)
            .is_some_and(|binding| !binding.settled)
        {
            tolerate_run_scoped(self.record_wire(run_id, host_epoch, &failure_fact()))?;
            tolerate_run_scoped(self.settle_if_open(run_id, host_epoch, DurableTurnPhase::Failed))?;
        }
        tolerate_run_scoped(self.release_steers(run_id, host_epoch))?;
        self.active.remove(run_id);
        Ok(())
    }

    pub(super) fn fail_dispatch(
        &self,
        prompt: &AgentChatPromptSaved,
        host_epoch: HostEpoch,
        error: RuntimeError,
    ) -> Result<CodexPromptDispatchOutcome, RuntimeError> {
        if is_daemon_fatal(&error) {
            return Err(error);
        }
        eprintln!(
            "Codex prompt {} failed before its turn started: {error}",
            prompt.message.message_id
        );
        self.runtime.fail_claimed_prompt(
            &prompt.message.message_id,
            &self.coordinator_id,
            host_epoch,
        )?;
        Ok(CodexPromptDispatchOutcome::Unprovable {
            run_id: prompt.run_id.0.clone(),
        })
    }
}
