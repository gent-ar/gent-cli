//! Private owner state for an approved Claude host.
//!
//! It is intentionally absent from daemon bootstrap and public capabilities. The owner only
//! recovers, ticks, drains, and signals an already authority-gated lifecycle; it never invents
//! provider terminal facts when a process cannot prove settlement.

use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, PublicProviderResolver, RunProjectionLedger,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;

use crate::approved_claude_host::{ApprovedClaudeDrain, ApprovedClaudeHost};
use crate::claude_prompt_lifecycle::ClaudeLifecycleTick;

/// State retained only by private Claude authority composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateClaudeSupervisorState {
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

/// Result of one bounded private owner wake.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateClaudeWake {
    Tick(ClaudeLifecycleTick),
    Drain(ApprovedClaudeDrain),
    Stopped,
    ShutdownRefused { active_runs: u16 },
}

/// Explicit shutdown result that does not create a terminal provider settlement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateClaudeShutdown {
    Stopped,
    Draining {
        active_runs: u16,
        signal: ProcessTreeSignal,
    },
    RefusedUndrained {
        active_runs: u16,
    },
}

/// Caller-timed process tree escalation without any implicit settlement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateClaudeEscalation {
    SignalSent(ProcessTreeSignal),
    NotDraining,
    RefusedUndrained { active_runs: u16 },
}

/// Bounded private owner for one already-approved Claude lifecycle host.
#[derive(Debug)]
pub(crate) struct PrivateClaudeSupervisor<L, D, R> {
    host: ApprovedClaudeHost<L, D, R>,
    state: PrivateClaudeSupervisorState,
}

impl<L, D, R> PrivateClaudeSupervisor<L, D, R>
where
    L: Clone
        + Ledger
        + RunProjectionLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger
        + AgentChatRunContextReader
        + ConversationContentReader,
    D: crate::claude_prompt_lifecycle::ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
    /// Binds a pre-approved host without recovery, launch, or process control.
    #[must_use]
    pub(crate) fn new(host: ApprovedClaudeHost<L, D, R>) -> Self {
        Self {
            host,
            state: PrivateClaudeSupervisorState::AwaitingRecovery,
        }
    }

    #[must_use]
    pub(crate) const fn state(&self) -> PrivateClaudeSupervisorState {
        self.state
    }

    /// Runs initial recovery once, then one bounded tick or drain for every private wake.
    ///
    /// # Errors
    /// Recovery failures leave this owner awaiting recovery and never launch a provider first.
    pub(crate) fn wake(&mut self) -> Result<PrivateClaudeWake, RuntimeError> {
        match self.state {
            PrivateClaudeSupervisorState::AwaitingRecovery => {
                self.host.recover()?;
                self.state = PrivateClaudeSupervisorState::Running;
            }
            PrivateClaudeSupervisorState::Stopped => return Ok(PrivateClaudeWake::Stopped),
            PrivateClaudeSupervisorState::ShutdownRefused { active_runs } => {
                return Ok(PrivateClaudeWake::ShutdownRefused { active_runs });
            }
            PrivateClaudeSupervisorState::ShutdownDraining { .. } => {
                let drain = self.host.drain()?;
                self.after_drain();
                return Ok(PrivateClaudeWake::Drain(drain));
            }
            PrivateClaudeSupervisorState::Running => {}
        }
        Ok(PrivateClaudeWake::Tick(self.host.tick()?))
    }

    /// Begins shutdown by interrupting every owned process tree exactly once.
    ///
    /// # Errors
    /// Returns an error if an owned process rejects the daemon-selected interrupt.
    pub(crate) fn request_shutdown(&mut self) -> Result<PrivateClaudeShutdown, RuntimeError> {
        if let PrivateClaudeSupervisorState::ShutdownDraining {
            active_runs,
            last_signal,
        } = self.state
        {
            return Ok(PrivateClaudeShutdown::Draining {
                active_runs,
                signal: last_signal,
            });
        }
        let active_runs = u16::try_from(self.host.active_len()).expect("host bound fits u16");
        if active_runs == 0 {
            self.state = PrivateClaudeSupervisorState::Stopped;
            return Ok(PrivateClaudeShutdown::Stopped);
        }
        if matches!(
            self.state,
            PrivateClaudeSupervisorState::ShutdownRefused { .. }
        ) {
            return Ok(PrivateClaudeShutdown::RefusedUndrained { active_runs });
        }
        self.host.signal_active(ProcessTreeSignal::Interrupt)?;
        self.state = PrivateClaudeSupervisorState::ShutdownDraining {
            active_runs,
            last_signal: ProcessTreeSignal::Interrupt,
        };
        Ok(PrivateClaudeShutdown::Draining {
            active_runs,
            signal: ProcessTreeSignal::Interrupt,
        })
    }

    /// Advances interrupt → terminate → kill only when its caller has waited for that deadline.
    ///
    /// # Errors
    /// Signal failures preserve the current state and cannot manufacture a terminal provider fact.
    pub(crate) fn escalate_shutdown(&mut self) -> Result<PrivateClaudeEscalation, RuntimeError> {
        let PrivateClaudeSupervisorState::ShutdownDraining {
            active_runs,
            last_signal,
        } = self.state
        else {
            return Ok(PrivateClaudeEscalation::NotDraining);
        };
        let Some(signal) = next_signal(last_signal) else {
            self.state = PrivateClaudeSupervisorState::ShutdownRefused { active_runs };
            return Ok(PrivateClaudeEscalation::RefusedUndrained { active_runs });
        };
        self.host.signal_active(signal)?;
        self.state = PrivateClaudeSupervisorState::ShutdownDraining {
            active_runs,
            last_signal: signal,
        };
        Ok(PrivateClaudeEscalation::SignalSent(signal))
    }

    fn after_drain(&mut self) {
        let active_runs = u16::try_from(self.host.active_len()).expect("host bound fits u16");
        if active_runs == 0 {
            self.state = PrivateClaudeSupervisorState::Stopped;
        } else if let PrivateClaudeSupervisorState::ShutdownDraining { last_signal, .. } =
            self.state
        {
            self.state = PrivateClaudeSupervisorState::ShutdownDraining {
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
#[path = "claude_authority_supervisor_tests.rs"]
mod tests;
