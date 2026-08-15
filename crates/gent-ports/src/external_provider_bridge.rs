//! Credential-free boundary for the private external-provider bridge.

use async_trait::async_trait;
use gent_types::{CapabilitySet, Command, DecisionCommand, ProviderEvent};

use crate::PortError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalProviderSession {
    pub run_id: String,
    pub opaque_session: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExternalProviderTerminal {
    Completed,
    Interrupted,
    Failed { message: String },
}

#[async_trait]
pub trait ExternalProviderBridge: Send + Sync {
    async fn register_capabilities(&self) -> Result<CapabilitySet, PortError> {
        Err(PortError::Unavailable("capability registration".into()))
    }
    async fn start_run(&self, run_id: &str) -> Result<ExternalProviderSession, PortError> {
        let _ = run_id;
        Err(PortError::Unavailable("run start".into()))
    }
    async fn resume_run(&self, run_id: &str) -> Result<ExternalProviderSession, PortError> {
        let _ = run_id;
        Err(PortError::Unavailable("run resume".into()))
    }
    async fn submit(&self, opaque_session: &str, command: Command) -> Result<(), PortError>;
    async fn answer_decision(
        &self,
        opaque_session: &str,
        decision: DecisionCommand,
    ) -> Result<(), PortError> {
        let _ = (opaque_session, decision);
        Err(PortError::Unavailable("decision submission".into()))
    }
    async fn interrupt(&self, opaque_session: &str) -> Result<(), PortError> {
        let _ = opaque_session;
        Err(PortError::Unavailable("interrupt".into()))
    }
    async fn next_event(&self, opaque_session: &str) -> Result<Option<ProviderEvent>, PortError>;
    async fn terminal_state(
        &self,
        opaque_session: &str,
    ) -> Result<Option<ExternalProviderTerminal>, PortError> {
        let _ = opaque_session;
        Err(PortError::Unavailable("terminal state".into()))
    }
}
