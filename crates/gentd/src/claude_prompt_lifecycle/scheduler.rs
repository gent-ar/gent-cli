//! Bounded daemon tick and recovery for the approved-only Claude lifecycle.

use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, PublicProviderResolver, RunProjectionLedger,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;

use super::{ClaudePromptDispatchOutcome, ClaudePromptLifecycle, ClaudePromptPoll};

impl<L, D, R> ClaudePromptLifecycle<L, D, R>
where
    L: Clone
        + Ledger
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + AgentChatRunContextReader
        + ConversationContentReader,
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
