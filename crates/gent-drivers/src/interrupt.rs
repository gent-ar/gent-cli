//! Pure interrupt escalation and process-tree control contract.

/// Signals must target the complete provider-owned process tree, never just a shell parent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessTreeSignal {
    Interrupt,
    Terminate,
    Kill,
}

/// I/O boundary for a platform-specific process-group or process-tree controller.
pub trait ProcessTreeControl: Send + Sync {
    /// Delivers a signal to the provider process tree.
    ///
    /// # Errors
    /// Returns an error when the operating-system controller cannot signal the tree.
    fn signal_tree(&self, signal: ProcessTreeSignal) -> Result<(), ProcessTreeError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessTreeError {
    #[error("process-tree signal failed: {0}")]
    Failed(String),
}

/// Timeouts used by the I/O owner to schedule the next escalation event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterruptPolicy {
    pub interrupt_grace_ms: u64,
    pub terminate_grace_ms: u64,
}

/// State retained by the orchestration edge for one provider process tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptState {
    Running,
    InterruptSent,
    TerminateSent,
    KillSent,
    Exited,
}

/// Facts reported by the timer or process monitor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptEvent {
    Requested,
    InterruptGraceElapsed,
    TerminateGraceElapsed,
    Exited,
}

/// A single prescribed side effect; the owner invokes it via [`ProcessTreeControl`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterruptTransition {
    pub state: InterruptState,
    pub signal: Option<ProcessTreeSignal>,
    pub next_wait_ms: Option<u64>,
}

/// Purely derives the next state and at most one process-tree signal.
#[must_use]
pub const fn transition(
    state: InterruptState,
    event: InterruptEvent,
    policy: InterruptPolicy,
) -> InterruptTransition {
    match (state, event) {
        (_, InterruptEvent::Exited) => InterruptTransition {
            state: InterruptState::Exited,
            signal: None,
            next_wait_ms: None,
        },
        (InterruptState::Running, InterruptEvent::Requested) => InterruptTransition {
            state: InterruptState::InterruptSent,
            signal: Some(ProcessTreeSignal::Interrupt),
            next_wait_ms: Some(policy.interrupt_grace_ms),
        },
        (InterruptState::InterruptSent, InterruptEvent::InterruptGraceElapsed) => {
            InterruptTransition {
                state: InterruptState::TerminateSent,
                signal: Some(ProcessTreeSignal::Terminate),
                next_wait_ms: Some(policy.terminate_grace_ms),
            }
        }
        (InterruptState::TerminateSent, InterruptEvent::TerminateGraceElapsed) => {
            InterruptTransition {
                state: InterruptState::KillSent,
                signal: Some(ProcessTreeSignal::Kill),
                next_wait_ms: None,
            }
        }
        _ => InterruptTransition {
            state,
            signal: None,
            next_wait_ms: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{InterruptEvent, InterruptPolicy, InterruptState, ProcessTreeSignal, transition};

    const POLICY: InterruptPolicy = InterruptPolicy {
        interrupt_grace_ms: 10,
        terminate_grace_ms: 20,
    };

    #[test]
    fn escalation_targets_the_tree_in_order() {
        let requested = transition(InterruptState::Running, InterruptEvent::Requested, POLICY);
        assert_eq!(requested.signal, Some(ProcessTreeSignal::Interrupt));
        let terminate = transition(
            requested.state,
            InterruptEvent::InterruptGraceElapsed,
            POLICY,
        );
        assert_eq!(terminate.signal, Some(ProcessTreeSignal::Terminate));
        let kill = transition(
            terminate.state,
            InterruptEvent::TerminateGraceElapsed,
            POLICY,
        );
        assert_eq!(kill.signal, Some(ProcessTreeSignal::Kill));
    }

    #[test]
    fn exit_wins_over_pending_escalation() {
        let transition = transition(
            InterruptState::InterruptSent,
            InterruptEvent::Exited,
            POLICY,
        );
        assert_eq!(transition.state, InterruptState::Exited);
        assert_eq!(transition.signal, None);
    }

    #[test]
    fn duplicate_requests_do_not_send_a_second_signal() {
        let transition = transition(
            InterruptState::InterruptSent,
            InterruptEvent::Requested,
            POLICY,
        );
        assert_eq!(transition.signal, None);
    }
}
