//! Private owner state for an approved Codex host.
//!
//! This contains no bootstrap, IPC, or authority selection. It makes the lifecycle's required
//! one-time recovery and bounded wake ticks explicit while refusing an unsafe shutdown that could
//! orphan provider processes or invent terminal facts.

use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;

use crate::approved_codex_host::{ApprovedCodexDrain, ApprovedCodexHost, ApprovedCodexTick};

/// State retained only by a private authority supervisor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCodexSupervisorState {
    AwaitingRecovery,
    Running,
    Stopped,
    ShutdownDraining {
        active_runs: u16,
        last_signal: ProcessTreeSignal,
    },
    ShutdownRefused {
        active_runs: u16,
    },
}

/// Result of one private supervisor wake.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCodexWake {
    Tick(ApprovedCodexTick),
    Drain(ApprovedCodexDrain),
    Stopped,
    ShutdownRefused { active_runs: u16 },
}

/// Explicit shutdown result that never manufactures provider terminal settlement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCodexShutdown {
    Stopped,
    Draining {
        active_runs: u16,
        signal: ProcessTreeSignal,
    },
    RefusedUndrained {
        active_runs: u16,
    },
}

/// Explicit process-tree escalation result; it does not settle a provider turn itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCodexEscalation {
    SignalSent(ProcessTreeSignal),
    NotDraining,
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
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + NormalizedSessionBatchLedger
        + AgentChatPromptDispatchLedger
        + gent_ports::AgentChatReadLedger
        + gent_ports::AgentChatRunContextReader
        + gent_ports::ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger,
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

    /// Returns whether recovery, active turn polling, or shutdown draining needs another wake.
    #[must_use]
    pub(crate) fn needs_drive(&self) -> bool {
        matches!(
            self.state,
            PrivateCodexSupervisorState::AwaitingRecovery
                | PrivateCodexSupervisorState::ShutdownDraining { .. }
        ) || (self.state == PrivateCodexSupervisorState::Running && self.host.needs_drive())
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
            PrivateCodexSupervisorState::ShutdownDraining { .. } => {
                let drain = self.host.drain()?;
                self.after_drain();
                return Ok(PrivateCodexWake::Drain(drain));
            }
            PrivateCodexSupervisorState::Running => {}
        }
        Ok(PrivateCodexWake::Tick(self.host.tick()?))
    }

    /// Starts a private drain only when the host can explicitly own every process tree signal.
    ///
    /// No provider terminal state is fabricated. A caller must wake/drain until the existing
    /// lifecycle has durably settled every active prompt, then request shutdown again.
    ///
    /// # Errors
    /// Returns an error if an owned process tree rejects the initial interrupt signal.
    pub(crate) fn request_shutdown(&mut self) -> Result<PrivateCodexShutdown, RuntimeError> {
        if let PrivateCodexSupervisorState::ShutdownDraining {
            active_runs,
            last_signal,
        } = self.state
        {
            return Ok(PrivateCodexShutdown::Draining {
                active_runs,
                signal: last_signal,
            });
        }
        let active_runs = u16::try_from(self.host.active_len()).expect("host bound fits u16");
        if active_runs == 0 {
            self.state = PrivateCodexSupervisorState::Stopped;
            return Ok(PrivateCodexShutdown::Stopped);
        }
        if matches!(
            self.state,
            PrivateCodexSupervisorState::ShutdownRefused { .. }
        ) {
            return Ok(PrivateCodexShutdown::RefusedUndrained { active_runs });
        }
        self.host.signal_active(ProcessTreeSignal::Interrupt)?;
        self.state = PrivateCodexSupervisorState::ShutdownDraining {
            active_runs,
            last_signal: ProcessTreeSignal::Interrupt,
        };
        Ok(PrivateCodexShutdown::Draining {
            active_runs,
            signal: ProcessTreeSignal::Interrupt,
        })
    }

    /// Advances an explicitly caller-timed interrupt → terminate → kill ladder.
    ///
    /// # Errors
    /// Returns an error when an owned process tree rejects the requested signal. The state stays
    /// unchanged, allowing the private owner to retry or continue polling without false facts.
    pub(crate) fn escalate_shutdown(&mut self) -> Result<PrivateCodexEscalation, RuntimeError> {
        let PrivateCodexSupervisorState::ShutdownDraining {
            active_runs,
            last_signal,
        } = self.state
        else {
            return Ok(PrivateCodexEscalation::NotDraining);
        };
        let Some(signal) = next_signal(last_signal) else {
            self.state = PrivateCodexSupervisorState::ShutdownRefused { active_runs };
            return Ok(PrivateCodexEscalation::RefusedUndrained { active_runs });
        };
        self.host.signal_active(signal)?;
        self.state = PrivateCodexSupervisorState::ShutdownDraining {
            active_runs,
            last_signal: signal,
        };
        Ok(PrivateCodexEscalation::SignalSent(signal))
    }

    fn after_drain(&mut self) {
        let active_runs = u16::try_from(self.host.active_len()).expect("host bound fits u16");
        if active_runs == 0 {
            self.state = PrivateCodexSupervisorState::Stopped;
        } else if let PrivateCodexSupervisorState::ShutdownDraining { last_signal, .. } = self.state
        {
            self.state = PrivateCodexSupervisorState::ShutdownDraining {
                active_runs,
                last_signal,
            };
        }
    }
}

fn next_signal(signal: ProcessTreeSignal) -> Option<ProcessTreeSignal> {
    match signal {
        ProcessTreeSignal::Interrupt => Some(ProcessTreeSignal::Terminate),
        ProcessTreeSignal::Terminate => Some(ProcessTreeSignal::Kill),
        ProcessTreeSignal::Kill => None,
    }
}

#[cfg(test)]
#[path = "codex_authority_supervisor_tests.rs"]
mod tests;
