//! Provider-neutral, private scheduling for one already-approved lifecycle owner.
//!
//! The loop is deliberately not a daemon service: its caller supplies every tick and chooses
//! when to request shutdown or escalate. It has a one-item mailbox so signals cannot pile up
//! ahead of drain work, and it never exposes mutable access to the approved owner.

/// Operations that a private lifecycle owner may perform on one caller-controlled tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateLifecycleCommand {
    Wake,
    RequestShutdown,
    EscalateShutdown,
}

/// Ordering state retained by the private loop, independent of provider-specific owner state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateLifecyclePhase {
    AwaitingWake,
    Ready,
    AwaitingDrainWake,
    ReadyToEscalate,
}

/// A rejected scheduling request never calls the owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateLifecycleScheduleError {
    Backpressured { pending: PrivateLifecycleCommand },
    WakeRequired,
    DrainWakeRequired,
}

/// Result from exactly one bounded owner call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PrivateLifecycleOutcome<W, S, E> {
    Wake(W),
    Shutdown(S),
    Escalation(E),
}

/// Private surface common to pre-approved Claude, Codex, or private bridge supervisors.
///
/// Implementors must keep each method bounded and must durably settle provider facts themselves.
/// This loop only serializes calls; it cannot infer provider terminal state.
pub(crate) trait PrivateLifecycleOwner {
    type Wake;
    type Shutdown;
    type Escalation;
    type Error;

    fn wake(&mut self) -> Result<Self::Wake, Self::Error>;
    fn request_shutdown(&mut self) -> Result<Self::Shutdown, Self::Error>;
    fn escalate_shutdown(&mut self) -> Result<Self::Escalation, Self::Error>;
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

/// One-slot mailbox and ordering fence around a single already-approved lifecycle owner.
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
    /// Binds an owner without recovering it, launching a provider, or scheduling a tick.
    #[must_use]
    pub(crate) fn new(owner: O) -> Self {
        Self {
            owner,
            pending: None,
            phase: PrivateLifecyclePhase::AwaitingWake,
        }
    }

    /// Returns the loop's ordering state; provider-specific state remains private to its owner.
    #[must_use]
    pub(crate) const fn phase(&self) -> PrivateLifecyclePhase {
        self.phase
    }

    /// Queues one caller-selected operation without executing it.
    ///
    /// A wake must prove recovery before shutdown. Every shutdown signal then needs an intervening
    /// wake/drain tick before another escalation, preserving process-tree backpressure.
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

    /// Executes at most one queued operation. No queued operation means no owner call.
    ///
    /// # Errors
    /// Returns the owner's error and preserves the prior phase, requiring an explicit later retry.
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
