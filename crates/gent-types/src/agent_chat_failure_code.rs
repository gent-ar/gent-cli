use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentChatFailureCode {
    SandboxUnavailable,
    SandboxAdmissionFailed,
    GentdRestarted,
    ProviderAuthRequired,
}

impl AgentChatFailureCode {
    #[must_use]
    pub const fn event_prefix(self) -> &'static str {
        match self {
            Self::SandboxUnavailable => "sandbox-unavailable:",
            Self::SandboxAdmissionFailed => "sandbox-admission-failed:",
            Self::GentdRestarted => "gentd-restarted:",
            Self::ProviderAuthRequired => "provider-auth-required:",
        }
    }

    #[must_use]
    pub fn from_event_id(event_id: &str) -> Option<Self> {
        [
            Self::SandboxUnavailable,
            Self::SandboxAdmissionFailed,
            Self::GentdRestarted,
            Self::ProviderAuthRequired,
        ]
        .into_iter()
        .find(|code| event_id.starts_with(code.event_prefix()))
    }
}
