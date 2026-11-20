use gent_drivers::codex_control::CodexControlRequest;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;

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
        + gent_ports::AttachmentLedger
        + gent_ports::AgentChatReadLedger
        + AgentChatRunContextReader
        + ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(super) fn admit_permission_request(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        request: CodexControlRequest,
    ) -> Result<u16, RuntimeError> {
        let ledger = self.runtime.ledger();
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        if crate::permission_queue::waits_behind(
            &ledger,
            &binding.prompt.message.conversation_id,
            run_id,
            &request.request_key,
            &binding.queued_permissions,
        )? {
            binding.queued_permissions.push_back(request);
            return Ok(0);
        }
        self.record_permission_request(run_id, host_epoch, request)
    }

    pub(super) fn retire_resolved_request(
        &mut self,
        run_id: &str,
        request_key: &str,
    ) -> Result<(), RuntimeError> {
        let ledger = self.runtime.ledger();
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        binding
            .queued_permissions
            .retain(|request| request.request_key != request_key);
        let pending = ledger.pending_permission(
            &gent_types::AgentChatConversationId(binding.prompt.message.conversation_id.clone()),
            &gent_types::AgentChatRunId(run_id.into()),
        )?;
        if let Some(pending) =
            pending.filter(|pending| pending.binding.decision_id.0 == request_key)
        {
            ledger.settle_pending_permission(&pending.binding)?;
        }
        Ok(())
    }

    pub(super) fn promote_queued_permission(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<Option<u16>, RuntimeError> {
        let ledger = self.runtime.ledger();
        let Some(binding) = self.active.get_mut(run_id) else {
            return Ok(None);
        };
        let conversation_id = binding.prompt.message.conversation_id.clone();
        match crate::permission_queue::next_ready(
            &ledger,
            &conversation_id,
            run_id,
            &mut binding.queued_permissions,
        )? {
            Some(request) => self
                .record_permission_request(run_id, host_epoch, request)
                .map(Some),
            None => Ok(None),
        }
    }
}
