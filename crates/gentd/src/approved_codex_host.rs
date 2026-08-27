//! Bounded approved-only Codex host composition, intentionally absent from daemon bootstrap.

use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;
use std::sync::Arc;

use crate::codex_prompt_lifecycle::{
    CodexPromptDispatchOutcome, CodexPromptLifecycle, CodexSummaryHook,
};
use crate::public_driver_runtime::PublicDriversRuntime;

/// Summary of one bounded approved-host scheduler pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ApprovedCodexTick {
    pub dispatch: Option<CodexPromptDispatchOutcome>,
    pub polled_runs: u16,
    pub facts: u16,
    pub exited_runs: u16,
}

/// Bounded process drain result without a new prompt claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ApprovedCodexDrain {
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
    D: crate::codex_prompt_lifecycle::CodexPromptExecution + Clone,
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
            lifecycle: CodexPromptLifecycle::new(runtime, coordinator_id),
            host_epoch,
            max_active: max_active.max(1),
        }
    }

    pub(crate) fn with_summary_hook(mut self, hook: Arc<dyn CodexSummaryHook>) -> Self {
        self.lifecycle = self.lifecycle.with_summary_hook(hook);
        self
    }

    /// Reconciles only old pre-launch durable dispatch claims before the first scheduler tick.
    pub(crate) fn recover(&self) -> Result<(), RuntimeError> {
        self.lifecycle.recover(self.host_epoch)
    }

    /// Returns the number of owned processes that would require an explicit drain on shutdown.
    #[must_use]
    pub(crate) fn active_len(&self) -> usize {
        self.lifecycle.active_len()
    }

    /// Returns whether an owned Codex turn needs another bounded polling pass.
    #[must_use]
    pub(crate) fn needs_drive(&self) -> bool {
        self.lifecycle.needs_poll()
    }

    /// Signals all currently owned provider process trees without claiming work.
    ///
    /// # Errors
    /// Returns an error when an owned process rejects the daemon-selected signal.
    pub(crate) fn signal_active(&self, signal: ProcessTreeSignal) -> Result<u16, RuntimeError> {
        self.lifecycle.signal_active(signal)
    }

    pub(crate) fn interrupt(&self, run_id: &str) -> Result<(), RuntimeError> {
        self.lifecycle.interrupt(run_id)
    }

    pub(crate) fn respond_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        self.lifecycle
            .respond_permission(run_id, request_id, decision, answers)
    }

    /// Drains at most the host active bound without accepting another prompt.
    ///
    /// # Errors
    /// Returns an error only when existing durable lifecycle polling fails.
    pub(crate) fn drain(&mut self) -> Result<ApprovedCodexDrain, RuntimeError> {
        let batch = self
            .lifecycle
            .poll_active(self.host_epoch, self.max_active)?;
        Ok(ApprovedCodexDrain {
            polled_runs: batch.polled_runs,
            facts: batch.facts,
            exited_runs: batch.exited_runs,
        })
    }

    /// Polls a bounded active snapshot before claiming at most one additional prompt.
    pub(crate) fn tick(&mut self) -> Result<ApprovedCodexTick, RuntimeError> {
        let batch = self.lifecycle.poll_active(self.host_epoch, self.max_active);
        let batch = batch?;
        let dispatch = (self.lifecycle.active_len() < self.max_active
            || self.lifecycle.has_settled_session())
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
