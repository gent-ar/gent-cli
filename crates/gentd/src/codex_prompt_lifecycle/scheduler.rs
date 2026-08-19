//! Bounded daemon tick and recovery for the approved-only Codex lifecycle.

use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;

use super::{CodexPromptDispatchOutcome, CodexPromptLifecycle};

const MAX_POLLS_PER_TICK: usize = 16;

/// One bounded scheduler pass, containing no provider-native frames.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexLifecycleTick {
    pub dispatch: CodexPromptDispatchOutcome,
    pub polled_runs: u16,
    pub facts: u16,
    pub exited_runs: u16,
}

/// Aggregate result of polling a bounded snapshot of owned Codex sessions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CodexPollBatch {
    pub polled_runs: u16,
    pub facts: u16,
    pub exited_runs: u16,
}

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
        + gent_ports::AgentChatWorkspaceLedger,
    D: super::CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    /// Recovers only work that is provably pre-launch under the new host epoch.
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
    ) -> Result<CodexPollBatch, RuntimeError> {
        let run_ids = self
            .active
            .keys()
            .take(maximum)
            .cloned()
            .collect::<Vec<_>>();
        let mut facts: u16 = 0;
        let mut exited_runs: u16 = 0;
        for run_id in &run_ids {
            if let Some(result) = self.poll(run_id, host_epoch)? {
                facts = facts.saturating_add(result.facts);
                exited_runs += u16::from(result.exited);
            }
        }
        Ok(CodexPollBatch {
            polled_runs: u16::try_from(run_ids.len()).expect("bounded poll count fits u16"),
            facts,
            exited_runs,
        })
    }

    /// Signals a stable active snapshot without accepting another durable prompt.
    ///
    /// # Errors
    /// Returns an error when any owned process tree rejects the daemon-selected signal.
    pub(crate) fn signal_active(&self, signal: ProcessTreeSignal) -> Result<u16, RuntimeError> {
        let run_ids = self.active.keys().cloned().collect::<Vec<_>>();
        for run_id in &run_ids {
            self.runner.signal_codex_process(run_id, signal)?;
        }
        Ok(u16::try_from(run_ids.len()).expect("active run count fits u16"))
    }

    /// Claims at most one prompt and polls at most sixteen owned sessions without blocking.
    pub(crate) fn tick(
        &mut self,
        host_epoch: HostEpoch,
    ) -> Result<CodexLifecycleTick, RuntimeError> {
        let dispatch = self.dispatch_next(host_epoch)?;
        let batch = self.poll_active(host_epoch, MAX_POLLS_PER_TICK)?;
        Ok(CodexLifecycleTick {
            dispatch,
            polled_runs: batch.polled_runs,
            facts: batch.facts,
            exited_runs: batch.exited_runs,
        })
    }
}
