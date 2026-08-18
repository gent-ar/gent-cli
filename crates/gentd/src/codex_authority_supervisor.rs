//! Private owner state for an approved Codex host.
//!
//! This contains no bootstrap, IPC, or authority selection. It makes the lifecycle's required
//! one-time recovery and bounded wake ticks explicit while refusing an unsafe shutdown that could
//! orphan provider processes or invent terminal facts.

use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, PublicProviderResolver,
    RunProjectionLedger, TranscriptLedger,
};
use gent_runtime::RuntimeError;

use crate::approved_codex_host::{ApprovedCodexHost, ApprovedCodexTick};

/// State retained only by a private authority supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCodexSupervisorState {
    AwaitingRecovery,
    Running,
    Stopped,
    ShutdownRefused { active_runs: u16 },
}

/// Result of one private supervisor wake.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCodexWake {
    Tick(ApprovedCodexTick),
    Stopped,
    ShutdownRefused { active_runs: u16 },
}

/// Explicit shutdown result that never manufactures provider terminal settlement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCodexShutdown {
    Stopped,
    RefusedUndrained { active_runs: u16 },
}

/// Bounded private owner for one already-approved Codex lifecycle host.
#[derive(Debug)]
pub(crate) struct PrivateCodexSupervisor<L, D, R> {
    host: ApprovedCodexHost<L, D, R>,
    state: PrivateCodexSupervisorState,
}

impl<L, D, R> PrivateCodexSupervisor<L, D, R>
where
    L: Clone
        + Ledger
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + gent_ports::AgentChatRunContextReader
        + gent_ports::ConversationContentReader,
    D: crate::codex_prompt_lifecycle::CodexPromptExecution + Clone,
    R: PublicProviderResolver,
{
    /// Binds an already authority-gated host without performing recovery or a process operation.
    #[must_use]
    pub(crate) fn new(host: ApprovedCodexHost<L, D, R>) -> Self {
        Self {
            host,
            state: PrivateCodexSupervisorState::AwaitingRecovery,
        }
    }

    /// Returns the explicit lifecycle owner state.
    #[must_use]
    pub(crate) const fn state(&self) -> PrivateCodexSupervisorState {
        self.state
    }

    /// Recovers once, then executes one bounded host tick for each private wake.
    ///
    /// # Errors
    /// Returns an error from recovery or a bounded host tick. Failed recovery is retried on a
    /// later wake; it never claims or starts a provider first.
    pub(crate) fn wake(&mut self) -> Result<PrivateCodexWake, RuntimeError> {
        match self.state {
            PrivateCodexSupervisorState::AwaitingRecovery => {
                self.host.recover()?;
                self.state = PrivateCodexSupervisorState::Running;
            }
            PrivateCodexSupervisorState::Stopped => return Ok(PrivateCodexWake::Stopped),
            PrivateCodexSupervisorState::ShutdownRefused { active_runs } => {
                return Ok(PrivateCodexWake::ShutdownRefused { active_runs });
            }
            PrivateCodexSupervisorState::Running => {}
        }
        Ok(PrivateCodexWake::Tick(self.host.tick()?))
    }

    /// Stops only when no owned provider process needs draining.
    ///
    /// An active process leaves the supervisor alive and refuses shutdown. A future composition
    /// must add a timer-driven process-tree drain and durable terminal settlement before replacing
    /// this refusal with an exit path.
    #[must_use]
    pub(crate) fn request_shutdown(&mut self) -> PrivateCodexShutdown {
        let active_runs = u16::try_from(self.host.active_len()).expect("host bound fits u16");
        if active_runs == 0 {
            self.state = PrivateCodexSupervisorState::Stopped;
            PrivateCodexShutdown::Stopped
        } else {
            self.state = PrivateCodexSupervisorState::ShutdownRefused { active_runs };
            PrivateCodexShutdown::RefusedUndrained { active_runs }
        }
    }
}

#[cfg(test)]
#[path = "codex_authority_supervisor_tests.rs"]
mod tests;
