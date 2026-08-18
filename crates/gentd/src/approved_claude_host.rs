//! Bounded approved-only Claude host composition, absent from daemon bootstrap.

use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, PublicProviderResolver, RunProjectionLedger,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;

use crate::claude_prompt_lifecycle::{
    ClaudeLifecycleTick, ClaudePromptExecution, ClaudePromptLifecycle,
};
use crate::public_driver_runtime::PublicDriversRuntime;

/// Process scheduler constructible only from an authority-gated public-driver runtime.
#[derive(Debug)]
pub(crate) struct ApprovedClaudeHost<L, D, R> {
    lifecycle: ClaudePromptLifecycle<L, D, R>,
    host_epoch: HostEpoch,
    max_active: usize,
}

impl<L, D, R> ApprovedClaudeHost<L, D, R>
where
    L: Clone
        + Ledger
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + AgentChatRunContextReader
        + ConversationContentReader,
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    #[must_use]
    pub(crate) fn new(
        runtime: PublicDriversRuntime<L, D, R>,
        coordinator_id: String,
        host_epoch: HostEpoch,
        max_active: usize,
    ) -> Self {
        Self {
            lifecycle: ClaudePromptLifecycle::new(runtime, coordinator_id),
            host_epoch,
            max_active: max_active.max(1),
        }
    }

    pub(crate) fn recover(&self) -> Result<(), RuntimeError> {
        self.lifecycle.recover(self.host_epoch)
    }

    pub(crate) fn tick(&mut self) -> Result<ClaudeLifecycleTick, RuntimeError> {
        self.lifecycle.tick(self.host_epoch, self.max_active)
    }
}
