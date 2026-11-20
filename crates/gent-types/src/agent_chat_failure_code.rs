use serde::{Deserialize, Serialize};

use crate::ProviderFailureClassification;

const RESETS_AT: &str = "resets-";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentChatFailureCode {
    SandboxUnavailable,
    SandboxAdmissionFailed,
    GentdRestarted,
    ProviderAuthRequired,
    ProviderRateLimited,
    ProviderContextLimit,
    ProviderOutputLimit,
    ProviderOverloaded,
    ProviderFailed,
}

impl AgentChatFailureCode {
    const ALL: [Self; 9] = [
        Self::SandboxUnavailable,
        Self::SandboxAdmissionFailed,
        Self::GentdRestarted,
        Self::ProviderAuthRequired,
        Self::ProviderRateLimited,
        Self::ProviderContextLimit,
        Self::ProviderOutputLimit,
        Self::ProviderOverloaded,
        Self::ProviderFailed,
    ];

    #[must_use]
    pub const fn event_prefix(self) -> &'static str {
        match self {
            Self::SandboxUnavailable => "sandbox-unavailable:",
            Self::SandboxAdmissionFailed => "sandbox-admission-failed:",
            Self::GentdRestarted => "gentd-restarted:",
            Self::ProviderAuthRequired => "provider-auth-required:",
            Self::ProviderRateLimited => "provider-rate-limited:",
            Self::ProviderContextLimit => "provider-context-limit:",
            Self::ProviderOutputLimit => "provider-output-limit:",
            Self::ProviderOverloaded => "provider-overloaded:",
            Self::ProviderFailed => "provider-failed:",
        }
    }

    #[must_use]
    pub const fn for_provider_failure(classification: ProviderFailureClassification) -> Self {
        match classification {
            ProviderFailureClassification::Authentication => Self::ProviderAuthRequired,
            ProviderFailureClassification::RateLimited => Self::ProviderRateLimited,
            ProviderFailureClassification::ContextLimit => Self::ProviderContextLimit,
            ProviderFailureClassification::OutputLimit => Self::ProviderOutputLimit,
            ProviderFailureClassification::Overloaded => Self::ProviderOverloaded,
            ProviderFailureClassification::Provider
            | ProviderFailureClassification::Lifecycle
            | ProviderFailureClassification::SessionUnavailable => Self::ProviderFailed,
        }
    }

    #[must_use]
    pub fn provider_failure_event_id(
        classification: ProviderFailureClassification,
        resets_at: Option<u64>,
        event_id: &str,
    ) -> String {
        let prefix = Self::for_provider_failure(classification).event_prefix();
        resets_at.map_or_else(
            || format!("{prefix}{event_id}"),
            |resets_at| format!("{prefix}{RESETS_AT}{resets_at}:{event_id}"),
        )
    }

    #[must_use]
    pub fn from_event_id(event_id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|code| event_id.starts_with(code.event_prefix()))
    }

    #[must_use]
    pub fn resets_at_from_event_id(event_id: &str) -> Option<u64> {
        event_id
            .strip_prefix(Self::ProviderRateLimited.event_prefix())?
            .strip_prefix(RESETS_AT)?
            .split_once(':')?
            .0
            .parse()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::AgentChatFailureCode;
    use crate::ProviderFailureClassification;

    #[test]
    fn every_provider_failure_names_a_code_and_a_rate_limit_keeps_its_reset() {
        let limited = AgentChatFailureCode::provider_failure_event_id(
            ProviderFailureClassification::RateLimited,
            Some(1_800_000_000),
            "event:7",
        );
        assert_eq!(
            AgentChatFailureCode::from_event_id(&limited),
            Some(AgentChatFailureCode::ProviderRateLimited)
        );
        assert_eq!(
            AgentChatFailureCode::resets_at_from_event_id(&limited),
            Some(1_800_000_000)
        );
        let failed = AgentChatFailureCode::provider_failure_event_id(
            ProviderFailureClassification::Lifecycle,
            None,
            "event:resets-9:x",
        );
        assert_eq!(
            AgentChatFailureCode::from_event_id(&failed),
            Some(AgentChatFailureCode::ProviderFailed)
        );
        assert_eq!(AgentChatFailureCode::resets_at_from_event_id(&failed), None);
    }
}
