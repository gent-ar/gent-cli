//! Bounded approved-only Claude host composition, absent from daemon bootstrap.

use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;
use gent_types::HostEpoch;
use std::sync::Arc;

use crate::claude_prompt_lifecycle::{
    ClaudeLifecycleTick, ClaudePromptExecution, ClaudePromptLifecycle, ClaudeSummaryHook,
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
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    #[must_use]
    pub(crate) fn new(
        runtime: PublicDriversRuntime<L, D, R>,
        coordinator_id: String,
        host_epoch: HostEpoch,
        max_active: usize,
        summary_hook: Option<Arc<dyn ClaudeSummaryHook>>,
    ) -> Self {
        Self {
            lifecycle: {
                let lifecycle = ClaudePromptLifecycle::new(runtime, coordinator_id);
                match summary_hook {
                    Some(hook) => lifecycle.with_summary_hook(hook),
                    None => lifecycle,
                }
            },
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

    /// Returns whether an owned Claude turn needs another bounded polling pass.
    #[must_use]
    pub(crate) fn needs_drive(&self) -> bool {
        self.lifecycle.needs_poll()
    }

    /// Signals every currently owned provider process tree without claiming work.
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
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), RuntimeError> {
        self.respond_permission_with_input(run_id, request_id, behavior, persist_suggestions, None)
    }

    pub(crate) fn respond_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        self.lifecycle.respond_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            updated_input,
        )
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
