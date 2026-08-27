//! Transient admission and shutdown control for one ordinary lifecycle cadence.
//!
//! This is deliberately neither a durable fact nor a recovery mechanism. The daemon uses it to
//! keep prompt admission closed until durable recovery has completely drained, and to close it
//! before beginning a graceful owner-proven shutdown.

use std::sync::{Arc, Mutex};

use tokio::sync::watch;

/// Process-local lifecycle phase visible to the future authority composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OrdinaryLifecyclePhase {
    Recovering,
    Ready,
    Draining,
}

/// Error returned when a prompt cannot enter an ordinary lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OrdinaryPromptAdmissionError {
    RecoveryInProgress,
    ShuttingDown,
}

#[derive(Clone, Copy, Debug)]
struct AdmissionState {
    phase: OrdinaryLifecyclePhase,
    permits: u32,
}

/// One transient authority latch shared by prompt admission and the cadence owner.
#[derive(Clone, Debug)]
pub(crate) struct OrdinaryLifecycleControl {
    state: Arc<Mutex<AdmissionState>>,
    phase: watch::Sender<OrdinaryLifecyclePhase>,
    permits: watch::Sender<u32>,
}

/// RAII lease held from before durable prompt submission through the lifecycle wake.
#[derive(Debug)]
pub(crate) struct OrdinaryPromptPermit {
    control: OrdinaryLifecycleControl,
}

impl OrdinaryLifecycleControl {
    /// Creates a closed latch. Only a fully drained recovery can open it.
    #[must_use]
    pub(crate) fn new() -> Self {
        let (phase, _) = watch::channel(OrdinaryLifecyclePhase::Recovering);
        let (permits, _) = watch::channel(0);
        Self {
            state: Arc::new(Mutex::new(AdmissionState {
                phase: OrdinaryLifecyclePhase::Recovering,
                permits: 0,
            })),
            phase,
            permits,
        }
    }

    /// Acquires the one transient permit that linearizes a prompt before its ledger transaction.
    pub(crate) fn acquire_prompt(
        &self,
    ) -> Result<OrdinaryPromptPermit, OrdinaryPromptAdmissionError> {
        let mut state = self
            .state
            .lock()
            .expect("ordinary lifecycle control is available");
        match state.phase {
            OrdinaryLifecyclePhase::Ready => Ok(()),
            OrdinaryLifecyclePhase::Recovering => {
                Err(OrdinaryPromptAdmissionError::RecoveryInProgress)
            }
            OrdinaryLifecyclePhase::Draining => Err(OrdinaryPromptAdmissionError::ShuttingDown),
        }?;
        state.permits = state.permits.saturating_add(1);
        self.permits.send_replace(state.permits);
        Ok(OrdinaryPromptPermit {
            control: self.clone(),
        })
    }

    /// Opens admission only when no shutdown request won the recovery race.
    pub(crate) fn open_after_recovery(&self) {
        let mut state = self
            .state
            .lock()
            .expect("ordinary lifecycle control is available");
        if state.phase == OrdinaryLifecyclePhase::Recovering {
            state.phase = OrdinaryLifecyclePhase::Ready;
            self.phase.send_replace(OrdinaryLifecyclePhase::Ready);
        }
    }

    /// Closes admission and asks the cadence to begin an owner-proven graceful drain.
    pub(crate) fn request_shutdown(&self) {
        let mut state = self
            .state
            .lock()
            .expect("ordinary lifecycle control is available");
        if state.phase != OrdinaryLifecyclePhase::Draining {
            state.phase = OrdinaryLifecyclePhase::Draining;
            self.phase.send_replace(OrdinaryLifecyclePhase::Draining);
        }
    }

    /// Returns the current transient lifecycle phase.
    #[must_use]
    pub(crate) fn phase(&self) -> OrdinaryLifecyclePhase {
        self.state
            .lock()
            .expect("ordinary lifecycle control is available")
            .phase
    }

    /// Waits until admission is closed for graceful shutdown.
    pub(crate) async fn shutdown_requested(&self) {
        let mut receiver = self.phase.subscribe();
        if *receiver.borrow() == OrdinaryLifecyclePhase::Draining {
            return;
        }
        while receiver.changed().await.is_ok() {
            if *receiver.borrow() == OrdinaryLifecyclePhase::Draining {
                return;
            }
        }
    }

    pub(crate) async fn wait_until_ready(&self) -> Result<(), OrdinaryPromptAdmissionError> {
        let mut receiver = self.phase.subscribe();
        loop {
            match *receiver.borrow() {
                OrdinaryLifecyclePhase::Ready => return Ok(()),
                OrdinaryLifecyclePhase::Draining => {
                    return Err(OrdinaryPromptAdmissionError::ShuttingDown);
                }
                OrdinaryLifecyclePhase::Recovering => {}
            }
            receiver
                .changed()
                .await
                .map_err(|_| OrdinaryPromptAdmissionError::RecoveryInProgress)?;
        }
    }

    /// Waits until all permits that began before shutdown have completed their typed exchange.
    pub(crate) async fn wait_for_permits(&self) {
        let mut receiver = self.permits.subscribe();
        if *receiver.borrow() == 0 {
            return;
        }
        while receiver.changed().await.is_ok() {
            if *receiver.borrow() == 0 {
                return;
            }
        }
    }
}

impl Drop for OrdinaryPromptPermit {
    fn drop(&mut self) {
        let mut state = self
            .control
            .state
            .lock()
            .expect("ordinary lifecycle control is available");
        state.permits = state.permits.saturating_sub(1);
        self.control.permits.send_replace(state.permits);
    }
}

#[cfg(test)]
#[path = "ordinary_lifecycle_control_tests.rs"]
mod tests;
