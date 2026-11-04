use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::AgentChatPromptSaved;

use super::{ClaudePromptExecution, ClaudePromptLifecycle};
use crate::provider_warm_pool::{IdleRun, WarmRun, evictions, expired};

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
    pub(super) fn make_room_for(
        &mut self,
        prompt: &AgentChatPromptSaved,
    ) -> Result<(), RuntimeError> {
        let evicted = evictions(
            self.pool_runs(),
            &prompt.run_id.0,
            &prompt.message.conversation_id,
            self.warm_capacity,
        );
        for run_id in evicted {
            self.release_owned(&run_id)?;
        }
        Ok(())
    }

    pub(super) fn pool_runs(&self) -> Vec<WarmRun<'_>> {
        self.active
            .iter()
            .map(|(run_id, binding)| WarmRun {
                run_id,
                conversation_id: &binding.prompt.message.conversation_id,
                idle: binding.idle(),
                last_dispatch: binding.last_dispatch,
            })
            .chain(self.warm.iter().map(|(run_id, warm)| WarmRun {
                run_id,
                conversation_id: &warm.conversation_id,
                idle: true,
                last_dispatch: warm.last_dispatch,
            }))
            .collect()
    }

    pub(super) fn release_owned(&mut self, run_id: &str) -> Result<(), RuntimeError> {
        if self.runner.has_claude_session(run_id) {
            self.runner.release_claude_session(run_id)?;
        }
        self.active.remove(run_id);
        self.warm.remove(run_id);
        Ok(())
    }

    pub(super) fn mark_dispatched(&mut self, run_id: &str) {
        self.dispatches = self.dispatches.saturating_add(1);
        if let Some(binding) = self.active.get_mut(run_id) {
            binding.last_dispatch = self.dispatches;
        }
    }

    pub(super) fn reap(&mut self, now: std::time::Instant) -> Result<(), RuntimeError> {
        let mut idle = Vec::new();
        for (run_id, binding) in &mut self.active {
            if binding.idle() {
                let since = *binding.idle_since.get_or_insert(now);
                let conversation_id = binding.prompt.message.conversation_id.clone();
                idle.push((run_id.clone(), conversation_id, since, true));
            } else {
                binding.idle_since = None;
            }
        }
        idle.extend(self.warm.iter().map(|(run_id, warm)| {
            (
                run_id.clone(),
                warm.conversation_id.clone(),
                warm.since,
                false,
            )
        }));
        let reaped = expired(
            idle.iter()
                .map(|(run_id, conversation_id, since, served)| IdleRun {
                    run_id,
                    idle_for: now.saturating_duration_since(*since),
                    served: *served,
                    current: self.is_current(conversation_id, run_id),
                }),
        );
        for run_id in reaped {
            self.release_owned(&run_id)?;
        }
        Ok(())
    }
}
