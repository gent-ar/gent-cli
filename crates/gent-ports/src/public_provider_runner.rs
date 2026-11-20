use gent_types::RunVersionLock;

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum PublicProviderRunError {
    #[error("public provider executable is not authorized by signed compatibility evidence")]
    CompatibilityDenied,
    #[error("provider executable changed before spawn or resume")]
    ProviderChanged,
    #[error("provider run is not active")]
    NotActive,
    #[error("permission answer input is invalid: {0}")]
    PermissionInputInvalid(String),
    #[error("provider lifecycle failed: {0}")]
    Failed(String),
    #[error("the sandbox is unavailable on this device: {0}")]
    SandboxUnavailable(String),
    #[error("the sandbox refused this launch: {0}")]
    SandboxAdmissionFailed(String),
}

impl PublicProviderRunError {
    #[must_use]
    pub const fn failure_code(&self) -> Option<gent_types::AgentChatFailureCode> {
        match self {
            Self::SandboxUnavailable(_) => {
                Some(gent_types::AgentChatFailureCode::SandboxUnavailable)
            }
            Self::SandboxAdmissionFailed(_) => {
                Some(gent_types::AgentChatFailureCode::SandboxAdmissionFailed)
            }
            _ => None,
        }
    }
}

pub trait PublicProviderRunner: Send + Sync {
    fn start(&self, run_id: &str, lock: &RunVersionLock) -> Result<(), PublicProviderRunError>;
    fn resume(
        &self,
        run_id: &str,
        lock: &RunVersionLock,
        session_id: &str,
    ) -> Result<(), PublicProviderRunError>;
    fn interrupt(&self, run_id: &str) -> Result<(), PublicProviderRunError>;
}
