use super::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ClaudePromptExecution,
    ClaudePromptLifecycle, ConversationActivityLedger, ConversationContentReader, HostEpoch,
    Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicDriverFact,
    PublicProviderResolver, RuntimeError, TranscriptLedger, missing_binding,
};
use crate::public_driver_runtime::NormalizedSessionFact;
use gent_drivers::public_protocol::PublicWireFact;

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
    pub(super) fn record_wire(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        fact: &PublicWireFact,
    ) -> Result<bool, RuntimeError> {
        if matches!(fact, PublicWireFact::SessionStarted { .. }) {
            let event_id = self.next_event_id(run_id, host_epoch, "session")?;
            self.runtime.record(
                run_id,
                &self.coordinator_id,
                host_epoch,
                PublicDriverFact::PublicWire {
                    event_id,
                    fact: fact.clone(),
                },
            )?;
            return Ok(false);
        }
        let binding = self
            .active
            .get(run_id)
            .cloned()
            .ok_or_else(missing_binding)?;
        let Some(fact) = crate::public_driver_runtime::session::recorded_wire_fact(
            &mut self.compaction_notices,
            &binding.prompt.message.turn_id,
            fact,
        ) else {
            return Ok(false);
        };
        let lifecycle_event_id = self.next_event_id(run_id, host_epoch, "wire")?;
        let transcript_event_id = self.next_event_id(run_id, host_epoch, "transcript")?;
        let activity_event_id = self.next_event_id(run_id, host_epoch, "activity")?;
        let input = NormalizedSessionFact {
            run_id: run_id.into(),
            conversation_id: binding.prompt.message.conversation_id,
            turn_id: binding.prompt.message.turn_id,
            host_epoch,
            lifecycle_event_id,
            transcript_event_id,
            activity_event_id,
            fact,
        };
        self.runtime
            .record_normalized_session(&self.coordinator_id, &input)?;
        Ok(false)
    }
}
