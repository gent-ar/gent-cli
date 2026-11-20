use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger,
    NormalizedSessionBatchLedger, PendingPermissionLedger, PolicyLedger, PublicProviderResolver,
    TranscriptLedger,
};
use gent_runtime::RuntimeError;

use crate::approved_codex_host::{ApprovedCodexDrain, ApprovedCodexHost, ApprovedCodexTick};

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCodexWake {
    Tick(ApprovedCodexTick),
    Drain(ApprovedCodexDrain),
    Stopped,
    ShutdownRefused { active_runs: u16 },
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateCodexEscalation {
    SignalSent(ProcessTreeSignal),
    NotDraining,
    RefusedUndrained { active_runs: u16 },
}

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
        + gent_ports::AttachmentLedger
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
    pub(crate) fn new(host: ApprovedCodexHost<L, D, R>) -> Self {
        Self {
            host,
            state: PrivateCodexSupervisorState::AwaitingRecovery,
        }
    }

    #[must_use]
    pub(crate) const fn state(&self) -> PrivateCodexSupervisorState {
        self.state
    }

    #[must_use]
    pub(crate) fn needs_drive(&self) -> bool {
        matches!(
            self.state,
            PrivateCodexSupervisorState::AwaitingRecovery
                | PrivateCodexSupervisorState::ShutdownDraining { .. }
        ) || (self.state == PrivateCodexSupervisorState::Running && self.host.needs_drive())
    }

    #[must_use]
    pub(crate) fn shutdown_complete(&self) -> bool {
        self.state == PrivateCodexSupervisorState::Stopped
    }

    pub(crate) fn respond_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        if self.state != PrivateCodexSupervisorState::Running {
            return Err(RuntimeError::ProviderRun(
                gent_ports::PublicProviderRunError::NotActive,
            ));
        }
        self.host
            .respond_permission(run_id, request_id, decision, answers)
    }

    pub(crate) fn tend_warm_pool(
        &mut self,
        request: &crate::provider_warm_pool::WarmPoolRequest,
    ) -> Result<Option<gent_protocol::ProviderWarmOutcome>, RuntimeError> {
        if self.state != PrivateCodexSupervisorState::Running {
            return Ok(request.unavailable());
        }
        self.host.tend_warm_pool(request)
    }

    pub(crate) fn interrupt_run(&mut self, run_id: &str) -> Result<(), RuntimeError> {
        if self.state != PrivateCodexSupervisorState::Running {
            return Err(RuntimeError::ProviderRun(
                gent_ports::PublicProviderRunError::Failed("Codex interrupt is unavailable".into()),
            ));
        }
        self.host.interrupt(run_id)
    }

    pub(crate) fn interrupt_turn(
        &mut self,
        run_id: &str,
        turn_id: &str,
    ) -> Result<(), RuntimeError> {
        if self.state != PrivateCodexSupervisorState::Running {
            return Err(RuntimeError::ProviderRun(
                gent_ports::PublicProviderRunError::Failed("interrupt owner is unavailable".into()),
            ));
        }
        self.host.interrupt_turn(run_id, turn_id)
    }

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
