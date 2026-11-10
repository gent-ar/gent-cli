use std::collections::VecDeque;

use gent_drivers::claude_control::{ClaudePermissionBehavior, ClaudePermissionRequest};
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
            return Ok(ClaudePermissionRecord {
                facts: 0,
                terminal: false,
            });
        }
        self.record_permission_request(run_id, host_epoch, request)
    }

    pub(super) fn promote_queued_permission(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<Option<ClaudePermissionRecord>, RuntimeError> {
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

    pub(super) fn deny_queued_permissions(
        &self,
        run_id: &str,
        queued: VecDeque<ClaudePermissionRequest>,
    ) {
        for request in queued {
            let _ = self.runner.respond_claude_permission_with_input(
                run_id,
                &request.request_id,
                ClaudePermissionBehavior::Deny,
                false,
                None,
            );
        }
    }
}
