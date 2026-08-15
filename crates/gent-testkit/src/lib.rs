//! Public fakes and fixture helpers for Gent contract tests.

use async_trait::async_trait;
use gent_ports::{ExternalProviderBridge, PortError};
use gent_types::{Command, ProviderEvent};
use std::sync::Mutex;

#[derive(Debug, Default)]
pub struct FakeExternalProviderBridge {
    submitted: Mutex<Vec<Command>>,
}

impl FakeExternalProviderBridge {
    /// Returns all commands observed by the fake.
    ///
    /// # Panics
    /// Panics only if a prior holder poisoned the test fake's mutex.
    pub fn submitted(&self) -> Vec<Command> {
        self.submitted
            .lock()
            .expect("fake bridge mutex poisoned")
            .clone()
    }
}

#[async_trait]
impl ExternalProviderBridge for FakeExternalProviderBridge {
    async fn submit(&self, _opaque_session: &str, command: Command) -> Result<(), PortError> {
        self.submitted
            .lock()
            .expect("fake bridge mutex poisoned")
            .push(command);
        Ok(())
    }

    async fn next_event(&self, _opaque_session: &str) -> Result<Option<ProviderEvent>, PortError> {
        Ok(None)
    }
}
