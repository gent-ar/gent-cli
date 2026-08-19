//! Bounded daemon tick and recovery for the approved-only Claude lifecycle.

use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;

use super::{ClaudePromptDispatchOutcome, ClaudePromptLifecycle, ClaudePromptPoll};

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
        + gent_ports::AgentChatWorkspaceLedger,
    D: super::ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    pub(crate) fn recover(&self, host_epoch: HostEpoch) -> Result<(), RuntimeError> {
        self.runtime.recover_prompts(host_epoch)
    }

    #[must_use]
    pub(crate) fn active_len(&self) -> usize {
        self.active.len()
    }

    /// Returns whether any owned run still needs stdout/exit polling.
    #[must_use]
    pub(crate) fn needs_poll(&self) -> bool {
        self.active.values().any(|binding| !binding.settled)
    }

    pub(crate) fn poll_active(
        &mut self,
        host_epoch: HostEpoch,
        maximum: usize,
    ) -> Result<ClaudePollBatch, RuntimeError> {
        let run_ids = self
            .active
            .keys()
            .take(maximum)
            .cloned()
            .collect::<Vec<_>>();
        let mut batch = ClaudePollBatch::default();
        for run_id in &run_ids {
            if let Some(ClaudePromptPoll { facts, exited }) = self.poll(run_id, host_epoch)? {
                batch.facts = batch.facts.saturating_add(facts);
                batch.exited_runs += u16::from(exited);
            }
        }
        batch.polled_runs = u16::try_from(run_ids.len()).expect("bounded poll count fits u16");
        Ok(batch)
    }

    /// Signals a stable active snapshot without accepting another durable prompt.
    ///
    /// # Errors
    /// Returns an error when an owned process tree rejects the daemon-selected signal.
    pub(crate) fn signal_active(&self, signal: ProcessTreeSignal) -> Result<u16, RuntimeError> {
        let run_ids = self.active.keys().cloned().collect::<Vec<_>>();
        for run_id in &run_ids {
            self.runner.signal_claude_process(run_id, signal)?;
        }
        Ok(u16::try_from(run_ids.len()).expect("active run count fits u16"))
    }

    pub(crate) fn tick(
        &mut self,
        host_epoch: HostEpoch,
        maximum: usize,
    ) -> Result<ClaudeLifecycleTick, RuntimeError> {
        let batch = self.poll_active(host_epoch, maximum)?;
        let dispatch = (self.active_len() < maximum)
            .then(|| self.dispatch_next(host_epoch))
            .transpose()?;
        Ok(ClaudeLifecycleTick { dispatch, batch })
    }
}

/// Aggregate result of one bounded Claude lifecycle pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeLifecycleTick {
    pub dispatch: Option<ClaudePromptDispatchOutcome>,
    pub batch: ClaudePollBatch,
}

/// Aggregate result of polling an owned Claude process snapshot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ClaudePollBatch {
    pub polled_runs: u16,
    pub facts: u16,
    pub exited_runs: u16,
}
