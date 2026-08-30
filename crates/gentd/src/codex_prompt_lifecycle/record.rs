use super::CodexPromptLifecycle;
use crate::public_driver_runtime::{NormalizedSessionFact, PublicDriverFact};
use gent_drivers::public_protocol::PublicWireFact;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;

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
    D: super::CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(super) fn record_wire(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        fact: &PublicWireFact,
    ) -> Result<(), RuntimeError> {
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
            return Ok(());
        }
        let binding = self
            .active
            .get(run_id)
            .cloned()
            .ok_or_else(super::missing_binding)?;
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
            fact: fact.clone(),
        };
        self.runtime
            .record_normalized_session(&self.coordinator_id, &input)?;
        Ok(())
    }

    pub(super) fn record_exit(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        code: Option<i32>,
    ) -> Result<(), RuntimeError> {
        let event_id = self.next_event_id(run_id, host_epoch, "exit")?;
        self.runtime.record(
            run_id,
            &self.coordinator_id,
            host_epoch,
            PublicDriverFact::SessionEffect {
                event_id,
                effect: gent_drivers::SessionEffect::Terminal {
                    reason: format!(
                        "providerExited:{}",
                        code.map_or_else(|| "unknown".into(), |value| value.to_string())
                    ),
                },
            },
        )?;
        Ok(())
    }

    pub(super) fn next_event_id(
        &mut self,
        run_id: &str,
        host_epoch: HostEpoch,
        kind: &str,
    ) -> Result<String, RuntimeError> {
        let binding = self
            .active
            .get_mut(run_id)
            .ok_or_else(super::missing_binding)?;
        binding.sequence = binding.sequence.saturating_add(1);
        Ok(format!(
            "codex:{}:{run_id}:{}:{kind}:{}",
            host_epoch.0, binding.prompt.message.turn_id, binding.sequence
        ))
    }
}
