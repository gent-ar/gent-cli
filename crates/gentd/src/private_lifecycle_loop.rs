#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateLifecycleCommand {
    Wake,
    RequestShutdown,
    EscalateShutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateLifecyclePhase {
    AwaitingWake,
    Ready,
    AwaitingDrainWake,
    ReadyToEscalate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateLifecycleScheduleError {
    Backpressured { pending: PrivateLifecycleCommand },
    WakeRequired,
    DrainWakeRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateLifecycleOutcome<W, S, E> {
    Wake(W),
    Shutdown(S),
    Escalation(E),
}

pub(crate) trait PrivateLifecycleOwner {
    type Wake;
    type Shutdown;
    type Escalation;
    type Error;

    fn wake(&mut self) -> Result<Self::Wake, Self::Error>;
    fn request_shutdown(&mut self) -> Result<Self::Shutdown, Self::Error>;
    fn escalate_shutdown(&mut self) -> Result<Self::Escalation, Self::Error>;

    fn needs_drive(&self) -> bool;

    fn shutdown_complete(&self) -> bool {
        false
    }

    fn respond_claude_permission(
        &self,
        _run_id: &str,
        _request_id: &str,
        _behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        _persist_suggestions: bool,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        Err(crate::ordinary_lifecycle_host::Undelivered::NotPending)
    }

    fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        let _ = updated_input;
        self.respond_claude_permission(run_id, request_id, behavior, persist_suggestions)
    }

    fn respond_codex_permission(
        &self,
        _run_id: &str,
        _request_id: &str,
        _decision: gent_drivers::codex_control::CodexControlDecision,
        _answers: Option<serde_json::Value>,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        Err(crate::ordinary_lifecycle_host::Undelivered::NotPending)
    }

    fn interrupt_run(&mut self, _: &str) -> Result<(), ()> {
        Err(())
    }
    fn tend_warm_pool(
        &mut self,
        request: &crate::provider_warm_pool::WarmPoolRequest,
    ) -> Result<Option<gent_protocol::ProviderWarmOutcome>, ()> {
        Ok(request.refused(gent_protocol::ProviderWarmOutcome::UnsupportedProvider))
    }
    fn interrupt_turn(&mut self, _: &str, _: &str) -> Result<(), ()> {
        Err(())
    }
}

type PrivateLifecycleTickResult<O> = Result<
    Option<
        PrivateLifecycleOutcome<
            <O as PrivateLifecycleOwner>::Wake,
            <O as PrivateLifecycleOwner>::Shutdown,
            <O as PrivateLifecycleOwner>::Escalation,
        >,
    >,
    <O as PrivateLifecycleOwner>::Error,
>;

#[derive(Debug)]
pub(crate) struct PrivateLifecycleLoop<O> {
    owner: O,
    pending: Option<PrivateLifecycleCommand>,
    phase: PrivateLifecyclePhase,
}

impl<O> PrivateLifecycleLoop<O>
where
    O: PrivateLifecycleOwner,
{
    #[must_use]
    pub(crate) fn new(owner: O) -> Self {
        Self {
            owner,
            pending: None,
            phase: PrivateLifecyclePhase::AwaitingWake,
        }
    }

    #[must_use]
    pub(crate) const fn phase(&self) -> PrivateLifecyclePhase {
        self.phase
    }

    #[must_use]
    pub(crate) fn owner_needs_drive(&self) -> bool {
        self.owner.needs_drive()
    }

    #[must_use]
    pub(crate) fn owner_shutdown_complete(&self) -> bool {
        self.owner.shutdown_complete()
    }

    pub(crate) fn respond_claude_permission(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        self.respond_claude_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            None,
        )
    }

    pub(crate) fn respond_claude_permission_with_input(
        &self,
        run_id: &str,
        request_id: &str,
        behavior: gent_drivers::claude_control::ClaudePermissionBehavior,
        persist_suggestions: bool,
        updated_input: Option<serde_json::Value>,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        self.owner.respond_claude_permission_with_input(
            run_id,
            request_id,
            behavior,
            persist_suggestions,
            updated_input,
        )
    }

    pub(crate) fn respond_codex_permission(
        &self,
        run_id: &str,
        request_id: &str,
        decision: gent_drivers::codex_control::CodexControlDecision,
        answers: Option<serde_json::Value>,
    ) -> Result<(), crate::ordinary_lifecycle_host::Undelivered> {
        self.owner
            .respond_codex_permission(run_id, request_id, decision, answers)
    }

    pub(crate) fn interrupt_run(&mut self, run_id: &str) -> Result<(), ()> {
        self.owner.interrupt_run(run_id)
    }
    pub(crate) fn tend_warm_pool(
        &mut self,
        request: &crate::provider_warm_pool::WarmPoolRequest,
    ) -> Result<Option<gent_protocol::ProviderWarmOutcome>, ()> {
        self.owner.tend_warm_pool(request)
    }
    pub(crate) fn interrupt_turn(&mut self, run_id: &str, turn_id: &str) -> Result<(), ()> {
        self.owner.interrupt_turn(run_id, turn_id)
    }

    pub(crate) fn schedule(
        &mut self,
        command: PrivateLifecycleCommand,
    ) -> Result<(), PrivateLifecycleScheduleError> {
        if let Some(pending) = self.pending {
            return Err(PrivateLifecycleScheduleError::Backpressured { pending });
        }
        match (self.phase, command) {
            (PrivateLifecyclePhase::AwaitingWake, PrivateLifecycleCommand::RequestShutdown) => {
                return Err(PrivateLifecycleScheduleError::WakeRequired);
            }
            (
                PrivateLifecyclePhase::AwaitingWake
                | PrivateLifecyclePhase::Ready
                | PrivateLifecyclePhase::AwaitingDrainWake,
                PrivateLifecycleCommand::EscalateShutdown,
            ) => {
                return Err(PrivateLifecycleScheduleError::DrainWakeRequired);
            }
            _ => {}
        }
        self.pending = Some(command);
        Ok(())
    }

    pub(crate) fn tick(&mut self) -> PrivateLifecycleTickResult<O> {
        let Some(command) = self.pending.take() else {
            return Ok(None);
        };
        let outcome = match command {
            PrivateLifecycleCommand::Wake => PrivateLifecycleOutcome::Wake(self.owner.wake()?),
            PrivateLifecycleCommand::RequestShutdown => {
                PrivateLifecycleOutcome::Shutdown(self.owner.request_shutdown()?)
            }
            PrivateLifecycleCommand::EscalateShutdown => {
                PrivateLifecycleOutcome::Escalation(self.owner.escalate_shutdown()?)
            }
        };
        self.phase = next_phase(self.phase, command);
        Ok(Some(outcome))
    }
}

const fn next_phase(
    phase: PrivateLifecyclePhase,
    command: PrivateLifecycleCommand,
) -> PrivateLifecyclePhase {
    match command {
        PrivateLifecycleCommand::Wake => match phase {
            PrivateLifecyclePhase::AwaitingWake => PrivateLifecyclePhase::Ready,
            PrivateLifecyclePhase::AwaitingDrainWake => PrivateLifecyclePhase::ReadyToEscalate,
            phase => phase,
        },
        PrivateLifecycleCommand::RequestShutdown | PrivateLifecycleCommand::EscalateShutdown => {
            PrivateLifecyclePhase::AwaitingDrainWake
        }
    }
}

#[cfg(test)]
#[path = "private_lifecycle_loop_tests.rs"]
mod tests;
