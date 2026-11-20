use crate::private_lifecycle_loop::{
    PrivateLifecycleCommand, PrivateLifecycleLoop, PrivateLifecycleOutcome, PrivateLifecycleOwner,
    PrivateLifecycleScheduleError,
};

impl<O> crate::agent_chat_api::PromptCommitWake for ProviderLifecycleHost<O>
where
    O: PrivateLifecycleOwner,
{
    type Error = ProviderLifecycleHostError<O::Error>;

    fn wake_after_prompt_commit(
        &mut self,
        _: crate::agent_chat_api::PromptWake,
    ) -> Result<(), Self::Error> {
        ProviderLifecycleWakePort::wake_after_prompt_commit(self).map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderLifecycleWake {
    Armed,
    Coalesced,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProviderLifecycleHostError<E> {
    Inactive,
    RecoveryRequired,
    ShutdownNotRequested,
    Schedule(PrivateLifecycleScheduleError),
    Owner(E),
}

pub(crate) type ProviderLifecycleHostOutcome<O> = PrivateLifecycleOutcome<
    <O as PrivateLifecycleOwner>::Wake,
    <O as PrivateLifecycleOwner>::Shutdown,
    <O as PrivateLifecycleOwner>::Escalation,
>;

pub(crate) trait ProviderLifecycleWakePort {
    type Error;

    fn wake_after_prompt_commit(&mut self) -> Result<ProviderLifecycleWake, Self::Error>;
}

#[derive(Debug)]
pub(crate) struct ProviderLifecycleHost<O> {
    lifecycle: PrivateLifecycleLoop<O>,
    armed: bool,
    shutdown_requested: bool,
}

impl<O> ProviderLifecycleHost<O>
where
    O: PrivateLifecycleOwner,
{
    #[must_use]
    pub(crate) fn new(owner: O) -> Self {
        Self {
            lifecycle: PrivateLifecycleLoop::new(owner),
            armed: false,
            shutdown_requested: false,
        }
    }

    #[must_use]
    pub(crate) const fn is_armed(&self) -> bool {
        self.armed
    }

    #[must_use]
    pub(crate) fn shutdown_complete(&self) -> bool {
        self.shutdown_requested && self.lifecycle.owner_shutdown_complete()
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
        self.lifecycle.respond_claude_permission_with_input(
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
        self.lifecycle
            .respond_codex_permission(run_id, request_id, decision, answers)
    }

    pub(crate) fn interrupt_run(&mut self, run_id: &str) -> Result<(), ()> {
        self.lifecycle.interrupt_run(run_id)
    }
    pub(crate) fn tend_warm_pool(
        &mut self,
        request: &crate::provider_warm_pool::WarmPoolRequest,
    ) -> Result<Option<gent_protocol::ProviderWarmOutcome>, ()> {
        self.lifecycle.tend_warm_pool(request)
    }
    pub(crate) fn interrupt_turn(&mut self, run_id: &str, turn_id: &str) -> Result<(), ()> {
        self.lifecycle.interrupt_turn(run_id, turn_id)
    }

    pub(crate) fn arm_authority_recovery(
        &mut self,
    ) -> Result<ProviderLifecycleWake, ProviderLifecycleHostError<O::Error>> {
        ProviderLifecycleWakePort::wake_after_prompt_commit(self)
    }

    pub(crate) fn drive(
        &mut self,
    ) -> Result<Option<ProviderLifecycleHostOutcome<O>>, ProviderLifecycleHostError<O::Error>> {
        if !self.armed {
            return Ok(None);
        }
        match self.lifecycle.schedule(PrivateLifecycleCommand::Wake) {
            Ok(()) | Err(PrivateLifecycleScheduleError::Backpressured { .. }) => {}
            Err(error) => return Err(ProviderLifecycleHostError::Schedule(error)),
        }
        let outcome = self
            .lifecycle
            .tick()
            .map_err(ProviderLifecycleHostError::Owner)?;
        self.armed = self.lifecycle.owner_needs_drive();
        Ok(outcome)
    }

    pub(crate) fn request_shutdown(&mut self) -> Result<(), ProviderLifecycleHostError<O::Error>> {
        self.schedule(PrivateLifecycleCommand::RequestShutdown)?;
        self.shutdown_requested = true;
        Ok(())
    }

    pub(crate) fn begin_shutdown_after_recovery(
        &mut self,
    ) -> Result<(), ProviderLifecycleHostError<O::Error>> {
        if matches!(
            self.lifecycle.phase(),
            crate::private_lifecycle_loop::PrivateLifecyclePhase::AwaitingWake
        ) {
            return Err(ProviderLifecycleHostError::RecoveryRequired);
        }
        if self.shutdown_requested {
            return Ok(());
        }

        let was_armed = self.armed;
        self.armed = true;
        if let Err(error) = self.schedule(PrivateLifecycleCommand::RequestShutdown) {
            self.armed = was_armed;
            return Err(error);
        }
        self.shutdown_requested = true;
        Ok(())
    }

    pub(crate) fn escalate_shutdown(&mut self) -> Result<(), ProviderLifecycleHostError<O::Error>> {
        if !self.shutdown_requested {
            return Err(ProviderLifecycleHostError::ShutdownNotRequested);
        }
        self.schedule(PrivateLifecycleCommand::EscalateShutdown)
    }

    fn schedule(
        &mut self,
        command: PrivateLifecycleCommand,
    ) -> Result<(), ProviderLifecycleHostError<O::Error>> {
        if !self.armed {
            return Err(ProviderLifecycleHostError::Inactive);
        }
        self.lifecycle
            .schedule(command)
            .map_err(ProviderLifecycleHostError::Schedule)
    }
}

impl<O> ProviderLifecycleWakePort for ProviderLifecycleHost<O>
where
    O: PrivateLifecycleOwner,
{
    type Error = ProviderLifecycleHostError<O::Error>;

    fn wake_after_prompt_commit(&mut self) -> Result<ProviderLifecycleWake, Self::Error> {
        let was_armed = self.armed;
        self.armed = true;
        match self.lifecycle.schedule(PrivateLifecycleCommand::Wake) {
            Ok(()) => Ok(ProviderLifecycleWake::Armed),
            Err(PrivateLifecycleScheduleError::Backpressured { .. }) if was_armed => {
                Ok(ProviderLifecycleWake::Coalesced)
            }
            Err(error) => Err(ProviderLifecycleHostError::Schedule(error)),
        }
    }
}

#[cfg(test)]
#[path = "provider_lifecycle_host_tests.rs"]
mod tests;
