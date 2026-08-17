//! Bounded approved-only Codex host composition, intentionally absent from daemon bootstrap.

use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, PublicProviderResolver,
    RunProjectionLedger, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;

use crate::codex_prompt_lifecycle::{CodexPromptDispatchOutcome, CodexPromptLifecycle};
use crate::public_driver_runtime::PublicDriversRuntime;

/// Summary of one bounded approved-host scheduler pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ApprovedCodexTick {
    pub dispatch: Option<CodexPromptDispatchOutcome>,
    pub polled_runs: u16,
    pub facts: u16,
    pub exited_runs: u16,
}

/// Process scheduler constructible only from an already authority-gated public-driver runtime.
#[derive(Debug)]
pub(crate) struct ApprovedCodexHost<L, D, R> {
    lifecycle: CodexPromptLifecycle<L, D, R>,
    host_epoch: HostEpoch,
    max_active: usize,
}

impl<L, D, R> ApprovedCodexHost<L, D, R>
where
    L: Clone
        + Ledger
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger,
    D: crate::codex_prompt_lifecycle::CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    #[must_use]
    pub(crate) fn new(
        runtime: PublicDriversRuntime<L, D, R>,
        coordinator_id: String,
        working_directory: Option<String>,
        host_epoch: HostEpoch,
        max_active: usize,
    ) -> Self {
        Self {
            lifecycle: CodexPromptLifecycle::new(runtime, coordinator_id, working_directory),
            host_epoch,
            max_active: max_active.max(1),
        }
    }

    /// Reconciles only old pre-launch durable dispatch claims before the first scheduler tick.
    pub(crate) fn recover(&self) -> Result<(), RuntimeError> {
        self.lifecycle.recover(self.host_epoch)
    }

    /// Polls a bounded active snapshot before claiming at most one additional prompt.
    pub(crate) fn tick(&mut self) -> Result<ApprovedCodexTick, RuntimeError> {
        let batch = self.lifecycle.poll_active(self.host_epoch, self.max_active);
        let batch = batch?;
        let dispatch = (self.lifecycle.active_len() < self.max_active)
            .then(|| self.lifecycle.dispatch_next(self.host_epoch))
            .transpose()?;
        Ok(ApprovedCodexTick {
            dispatch,
            polled_runs: batch.polled_runs,
            facts: batch.facts,
            exited_runs: batch.exited_runs,
        })
    }
}
