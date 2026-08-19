//! Private daemon-composition owner for one approved provider lifecycle.
//!
//! This is not bootstrap, IPC, or a capability. A future approved composition calls the wake
//! port only after the prompt transaction commits. Until then it cannot recover, poll, drain, or
//! signal its owner. The host deliberately retains no provider-session snapshot: recovery stays
//! behind the owner and its durable cursor ledger.

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

/// Result of notifying a private host that at least one prompt is durably committed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderLifecycleWake {
    Armed,
    Coalesced,
}

/// Private host errors never cross IPC or contain provider-native data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProviderLifecycleHostError<E> {
    Inactive,
    RecoveryRequired,
    ShutdownNotRequested,
    Schedule(PrivateLifecycleScheduleError),
    Owner(E),
}

/// Outcome of at most one bounded private owner operation.
pub(crate) type ProviderLifecycleHostOutcome<O> = PrivateLifecycleOutcome<
    <O as PrivateLifecycleOwner>::Wake,
    <O as PrivateLifecycleOwner>::Shutdown,
    <O as PrivateLifecycleOwner>::Escalation,
>;

/// Post-commit notification boundary retained by a future daemon composition only.
///
/// Calling this port does not launch a provider. It arms one pre-approved owner so a later
/// bounded [`ProviderLifecycleHost::drive`] can recover or poll it. Repeated committed prompts
/// are coalesced while a wake is already queued, preserving backpressure without dropping the
/// durable work that prompted the notification. The owner declares whether it needs another
/// drive, so settled sessions do not cause blind background polling.
pub(crate) trait ProviderLifecycleWakePort {
    type Error;

    fn wake_after_prompt_commit(&mut self) -> Result<ProviderLifecycleWake, Self::Error>;
}

/// Generic private owner for an already-approved Claude, Codex, or bridge lifecycle.
///
/// No default daemon composition creates this value. Once armed, each caller-timed drive performs
/// at most one owner call. The caller supplies the cadence, while the existing one-item loop
/// enforces recovery, drain, and signal ordering.
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
    /// Binds a pre-approved owner while keeping every lifecycle operation inactive.
    #[must_use]
    pub(crate) fn new(owner: O) -> Self {
        Self {
            lifecycle: PrivateLifecycleLoop::new(owner),
            armed: false,
            shutdown_requested: false,
        }
    }

    /// Returns whether a durable post-commit notification has armed this private host.
    #[must_use]
    pub(crate) const fn is_armed(&self) -> bool {
        self.armed
    }

    /// Returns whether a requested shutdown has fully drained the owned process tree.
    ///
    /// An untouched host is deliberately not considered stopped: a caller must first prove
    /// recovery before it can request shutdown. This preserves the distinction between an owner
    /// that never inspected durable work and one that finished an explicit drain.
    #[must_use]
    pub(crate) const fn shutdown_complete(&self) -> bool {
        self.shutdown_requested && !self.armed
    }

    /// Arms the owner for its one durable startup recovery pass.
    ///
    /// This does not inspect or launch a provider. A daemon-owned cadence must later call
    /// [`Self::drive`], whose owner recovery reclaims only durable pre-launch work.
    pub(crate) fn arm_authority_recovery(
        &mut self,
    ) -> Result<ProviderLifecycleWake, ProviderLifecycleHostError<O::Error>> {
        ProviderLifecycleWakePort::wake_after_prompt_commit(self)
    }

    /// Drives exactly one recovery, polling, drain, or signal operation after arming.
    ///
    /// When no control command is pending, a drive requests one ordinary wake. A pending control
    /// command wins, so shutdown and escalation retain the loop's bounded backpressure ordering.
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

    /// Queues one process-tree drain request after the host has been armed.
    pub(crate) fn request_shutdown(&mut self) -> Result<(), ProviderLifecycleHostError<O::Error>> {
        self.schedule(PrivateLifecycleCommand::RequestShutdown)?;
        self.shutdown_requested = true;
        Ok(())
    }

    /// Starts process-tree shutdown only after recovery has completed at least one wake.
    ///
    /// A recovered idle owner has no pending work and is normally unarmed. This method arms it
    /// only to carry the shutdown command; it never manufactures a wake, because such a wake
    /// could be mistaken for a durable prompt. A fresh owner remains in `AwaitingWake` and is
    /// rejected instead of being implicitly recovered or shut down.
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

    /// Queues the next explicit shutdown escalation after an intervening drain wake.
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
