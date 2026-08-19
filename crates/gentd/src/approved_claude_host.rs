//! Bounded approved-only Claude host composition, absent from daemon bootstrap.

use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PublicProviderResolver,
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

/// Bounded process drain result without a new prompt claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ApprovedClaudeDrain {
    pub polled_runs: u16,
    pub facts: u16,
    pub exited_runs: u16,
}

impl<L, D, R> ApprovedClaudeHost<L, D, R>
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
        + gent_ports::AgentChatWorkspaceLedger,
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

    /// Returns the number of owned processes requiring explicit drain on shutdown.
    #[must_use]
    pub(crate) fn active_len(&self) -> usize {
        self.lifecycle.active_len()
    }

    /// Signals every currently owned provider process tree without claiming work.
    ///
    /// # Errors
    /// Returns an error when an owned process rejects the daemon-selected signal.
    pub(crate) fn signal_active(&self, signal: ProcessTreeSignal) -> Result<u16, RuntimeError> {
        self.lifecycle.signal_active(signal)
    }

    /// Drains the current bounded owned-process snapshot without accepting a prompt.
    ///
    /// # Errors
    /// Returns an error only when existing durable lifecycle polling fails.
    pub(crate) fn drain(&mut self) -> Result<ApprovedClaudeDrain, RuntimeError> {
        let batch = self
            .lifecycle
            .poll_active(self.host_epoch, self.max_active)?;
        Ok(ApprovedClaudeDrain {
            polled_runs: batch.polled_runs,
            facts: batch.facts,
            exited_runs: batch.exited_runs,
        })
    }

    pub(crate) fn tick(&mut self) -> Result<ClaudeLifecycleTick, RuntimeError> {
        self.lifecycle.tick(self.host_epoch, self.max_active)
    }
}
