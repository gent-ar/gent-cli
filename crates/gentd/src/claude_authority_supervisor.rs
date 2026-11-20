use gent_drivers::interrupt::ProcessTreeSignal;
use gent_ports::{
    AgentChatPromptDispatchLedger, AgentChatRunContextReader, ConversationActivityLedger,
    ConversationContentReader, Ledger, NormalizedSessionBatchLedger, PendingPermissionLedger,
    PolicyLedger, PublicProviderResolver, TranscriptLedger,
};
use gent_runtime::RuntimeError;

use crate::approved_claude_host::{ApprovedClaudeDrain, ApprovedClaudeHost};
use crate::claude_prompt_lifecycle::ClaudeLifecycleTick;

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateClaudeWake {
    Tick(ClaudeLifecycleTick),
    Drain(ApprovedClaudeDrain),
    Stopped,
    ShutdownRefused { active_runs: u16 },
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateClaudeEscalation {
    SignalSent(ProcessTreeSignal),
    NotDraining,
    RefusedUndrained { active_runs: u16 },
}

#[derive(Debug)]
pub(crate) struct PrivateClaudeSupervisor<L, D, R> {
    host: ApprovedClaudeHost<L, D, R>,
    state: PrivateClaudeSupervisorState,
}

impl<L, D, R> PrivateClaudeSupervisor<L, D, R>
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
        + AgentChatRunContextReader
        + ConversationContentReader
        + gent_ports::AgentChatWorkspaceLedger
        + PendingPermissionLedger
        + PolicyLedger
        + gent_ports::AttachmentLedger
        + gent_ports::ToolSourceLedger
        + gent_ports::AgentChatConversationConfigLedger,
    D: crate::claude_prompt_lifecycle::ClaudePromptExecution + Clone,
    R: PublicProviderResolver,
{
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

    #[must_use]
    pub(crate) fn needs_drive(&self) -> bool {
        matches!(
            self.state,
            PrivateClaudeSupervisorState::AwaitingRecovery
                | PrivateClaudeSupervisorState::ShutdownDraining { .. }
        ) || (self.state == PrivateClaudeSupervisorState::Running && self.host.needs_drive())
    }

    #[must_use]
    pub(crate) fn shutdown_complete(&self) -> bool {
        self.state == PrivateClaudeSupervisorState::Stopped
    }

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
        if self.state != PrivateClaudeSupervisorState::Running {
            return Err(RuntimeError::ProviderRun(
                gent_ports::PublicProviderRunError::NotActive,
            ));
        }
        self.host.respond_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            updated_input,
        )
    }

    pub(crate) fn tend_warm_pool(
        &mut self,
        request: &crate::provider_warm_pool::WarmPoolRequest,
    ) -> Result<Option<gent_protocol::ProviderWarmOutcome>, RuntimeError> {
        if self.state != PrivateClaudeSupervisorState::Running {
            return Ok(request.unavailable());
        }
        self.host.tend_warm_pool(request)
    }

    pub(crate) fn interrupt_run(&mut self, run_id: &str) -> Result<(), RuntimeError> {
        if self.state != PrivateClaudeSupervisorState::Running {
            return Err(RuntimeError::ProviderRun(
                gent_ports::PublicProviderRunError::Failed(
                    "Claude interrupt is unavailable".into(),
                ),
            ));
        }
        self.host.interrupt(run_id)
    }

    pub(crate) fn interrupt_turn(
        &mut self,
        run_id: &str,
        turn_id: &str,
    ) -> Result<(), RuntimeError> {
        if self.state != PrivateClaudeSupervisorState::Running {
            return Err(RuntimeError::ProviderRun(
                gent_ports::PublicProviderRunError::Failed("interrupt owner is unavailable".into()),
            ));
        }
        self.host.interrupt_turn(run_id, turn_id)
    }

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
