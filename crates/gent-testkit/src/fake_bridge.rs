//! Deterministic public implementation of the private provider-bridge port.
#![allow(clippy::missing_panics_doc)] // Test fakes fail fast on poisoned state.

use async_trait::async_trait;
use gent_ports::{
    ExternalProviderBridge, ExternalProviderSession, ExternalProviderTerminal, PortError,
};
use gent_types::{CapabilitySet, Command, DecisionCommand, ProviderEvent};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq)]
pub struct BridgeSubmission {
    pub opaque_session: String,
    pub command: Command,
}

#[derive(Debug, Default)]
struct BridgeState {
    capabilities: CapabilitySet,
    sessions: Vec<ExternalProviderSession>,
    interrupts: Vec<String>,
    submitted: Vec<BridgeSubmission>,
    events: VecDeque<Result<ProviderEvent, String>>,
    submit_failures: VecDeque<String>,
}

/// A scriptable bridge fake. Each configured event or failure is consumed once,
/// making test outcomes independent of timing or provider availability.
#[derive(Debug, Default)]
pub struct FakeExternalProviderBridge {
    state: Mutex<BridgeState>,
}

impl FakeExternalProviderBridge {
    pub fn set_capabilities(&self, capabilities: CapabilitySet) {
        self.state
            .lock()
            .expect("fake bridge mutex poisoned")
            .capabilities = capabilities;
    }

    #[must_use]
    pub fn sessions(&self) -> Vec<ExternalProviderSession> {
        self.state
            .lock()
            .expect("fake bridge mutex poisoned")
            .sessions
            .clone()
    }
    /// Queues one event for the next `next_event` call.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    pub fn push_event(&self, event: ProviderEvent) {
        self.state
            .lock()
            .expect("fake bridge mutex poisoned")
            .events
            .push_back(Ok(event));
    }

    /// Queues one provider failure for the next `next_event` call.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    pub fn fail_next_event(&self, message: impl Into<String>) {
        self.state
            .lock()
            .expect("fake bridge mutex poisoned")
            .events
            .push_back(Err(message.into()));
    }

    /// Queues one provider failure for the next `submit` call.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    pub fn fail_next_submit(&self, message: impl Into<String>) {
        self.state
            .lock()
            .expect("fake bridge mutex poisoned")
            .submit_failures
            .push_back(message.into());
    }

    /// Returns submissions in the order the bridge observed them.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    #[must_use]
    pub fn submissions(&self) -> Vec<BridgeSubmission> {
        self.state
            .lock()
            .expect("fake bridge mutex poisoned")
            .submitted
            .clone()
    }

    /// Retained for existing tests that only need submitted commands.
    ///
    /// # Panics
    /// Panics if a test has poisoned the fake's mutex.
    #[must_use]
    pub fn submitted(&self) -> Vec<Command> {
        self.submissions()
            .into_iter()
            .map(|submission| submission.command)
            .collect()
    }
}

#[async_trait]
impl ExternalProviderBridge for FakeExternalProviderBridge {
    async fn register_capabilities(&self) -> Result<CapabilitySet, PortError> {
        Ok(self
            .state
            .lock()
            .expect("fake bridge mutex poisoned")
            .capabilities
            .clone())
    }

    async fn start_run(&self, run_id: &str) -> Result<ExternalProviderSession, PortError> {
        let session = ExternalProviderSession {
            run_id: run_id.into(),
            opaque_session: format!("bridge-{run_id}"),
        };
        self.state
            .lock()
            .expect("fake bridge mutex poisoned")
            .sessions
            .push(session.clone());
        Ok(session)
    }

    async fn resume_run(&self, run_id: &str) -> Result<ExternalProviderSession, PortError> {
        self.start_run(run_id).await
    }
    async fn submit(&self, opaque_session: &str, command: Command) -> Result<(), PortError> {
        let mut state = self.state.lock().expect("fake bridge mutex poisoned");
        state.submitted.push(BridgeSubmission {
            opaque_session: opaque_session.into(),
            command,
        });
        state
            .submit_failures
            .pop_front()
            .map_or(Ok(()), |message| Err(PortError::Provider(message)))
    }

    async fn answer_decision(
        &self,
        _opaque_session: &str,
        _decision: DecisionCommand,
    ) -> Result<(), PortError> {
        Ok(())
    }

    async fn interrupt(&self, opaque_session: &str) -> Result<(), PortError> {
        self.state
            .lock()
            .expect("fake bridge mutex poisoned")
            .interrupts
            .push(opaque_session.into());
        Ok(())
    }

    async fn next_event(&self, _opaque_session: &str) -> Result<Option<ProviderEvent>, PortError> {
        self.state
            .lock()
            .expect("fake bridge mutex poisoned")
            .events
            .pop_front()
            .transpose()
            .map_err(PortError::Provider)
    }

    async fn terminal_state(
        &self,
        _opaque_session: &str,
    ) -> Result<Option<ExternalProviderTerminal>, PortError> {
        Ok(None)
    }
}
