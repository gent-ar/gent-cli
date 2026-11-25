use gent_drivers::claude_control::ClaudePermissionRequest;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;

use super::permission::ClaudePermissionRecord;
use super::{ClaudePromptExecution, ClaudePromptLifecycle, missing_binding};

impl<L, D, R> ClaudePromptLifecycle<L, D, R>
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
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(super) fn admit_permission_request(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        request: ClaudePermissionRequest,
    ) -> Result<ClaudePermissionRecord, RuntimeError> {
        let ledger = self.runtime.ledger();
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        if crate::permission_queue::waits_behind(
            &ledger,
            &binding.prompt.message.conversation_id,
            run_id,
            &request.request_id,
            &binding.queued_permissions,
        )? {
            binding.queued_permissions.push_back(request);
            return Ok(ClaudePermissionRecord { facts: 0 });
        }
        self.record_permission_request(run_id, host_epoch, request)
    }

    pub(super) fn promote_queued_permission(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<Option<ClaudePermissionRecord>, RuntimeError> {
        let ledger = self.runtime.ledger();
        let Some(binding) = self.active.get(run_id) else {
            return Ok(None);
        };
        let conversation_id = binding.prompt.message.conversation_id.clone();
        self.reconsider_pending_permission(run_id, &conversation_id, host_epoch)?;
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
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

    fn reconsider_pending_permission(
        &mut self,
        run_id: &str,
        conversation_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        let ledger = self.runtime.ledger();
        let Some(pending) = crate::permission_queue::pending(&ledger, conversation_id, run_id)?
        else {
            return Ok(());
        };
        let policy = self.runtime.run_policy(conversation_id, run_id)?;
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        if !crate::permission_queue::reconsiders(
            &pending,
            &policy,
            &mut binding.reconsidered_revision,
        ) {
            return Ok(());
        }
        let Some(automatic) =
            self.automatic_permission(run_id, host_epoch, &policy, &pending.request)?
        else {
            return Ok(());
        };
        self.runner.respond_claude_permission_with_input(
            run_id,
            &pending.binding.decision_id.0,
            super::permission::behavior_for(automatic),
            false,
            None,
        )?;
        ledger.settle_delivered_permission(&pending.binding, None, automatic.outcome())?;
        Ok(())
    }

    pub(super) fn permission_cancelled(
        &mut self,
        run_id: &str,
        request_id: &str,
    ) -> Result<(), RuntimeError> {
        let ledger = self.runtime.ledger();
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        binding
            .queued_permissions
            .retain(|request| request.request_id != request_id);
        let conversation_id =
            gent_types::AgentChatConversationId(binding.prompt.message.conversation_id.clone());
        if let Some(pending) = ledger
            .pending_permission(&conversation_id, &gent_types::AgentChatRunId(run_id.into()))?
            .filter(|pending| pending.binding.decision_id.0 == request_id)
        {
            ledger.settle_pending_permission(&pending.binding)?;
        }
        Ok(())
    }

    pub(super) fn track_child(
        &mut self,
        run_id: &str,
        fact: &gent_drivers::public_protocol::PublicWireFact,
    ) -> Result<(), RuntimeError> {
        use gent_drivers::public_protocol::PublicWireFact::Event;
        use gent_types::NormalizedProviderEvent::{ChildStarted, ChildTerminal};
        let binding = self.active.get_mut(run_id).ok_or_else(missing_binding)?;
        match fact {
            Event(ChildStarted { child_id, .. }) => {
                binding.background_children.insert(child_id.clone());
            }
            Event(ChildTerminal { child_id, .. }) => {
                binding.background_children.remove(child_id);
                binding
                    .queued_permissions
                    .retain(|request| request.child_id.as_ref() != Some(child_id));
            }
            _ => {}
        }
        Ok(())
    }
}
