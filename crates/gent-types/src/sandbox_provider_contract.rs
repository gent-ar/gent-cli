use crate::{AgentChatProvider, SandboxGatewayApi};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContainedProviderRoute {
    ManagedPackage,
    ClaurstStandalone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContainedProviderContract {
    pub provider_id: &'static str,
    pub gateway_api: SandboxGatewayApi,
    pub route: ContainedProviderRoute,
    pub credential_contracts: &'static [&'static str],
}

impl AgentChatProvider {
    #[must_use]
    pub const fn contained_contract(self) -> ContainedProviderContract {
        match self {
            Self::Claude => ContainedProviderContract {
                provider_id: "claude",
                gateway_api: SandboxGatewayApi::Claude,
                route: ContainedProviderRoute::ManagedPackage,
                credential_contracts: &["claude-loopback-v1"],
            },
            Self::Codex => ContainedProviderContract {
                provider_id: "codex",
                gateway_api: SandboxGatewayApi::Codex,
                route: ContainedProviderRoute::ManagedPackage,
                credential_contracts: &["codex-file-responses-v1"],
            },
            Self::Claurst => ContainedProviderContract {
                provider_id: "claurst",
                gateway_api: SandboxGatewayApi::Claurst,
                route: ContainedProviderRoute::ClaurstStandalone,
                credential_contracts: &["claurst-daemon-model-v1", "claurst-daemon-model-mcp-v1"],
            },
        }
    }

    #[must_use]
    pub fn from_contained_id(id: &str) -> Option<Self> {
        [Self::Claude, Self::Codex, Self::Claurst]
            .into_iter()
            .find(|provider| provider.contained_contract().provider_id == id)
    }
}

impl SandboxGatewayApi {
    #[must_use]
    pub const fn provider(self) -> AgentChatProvider {
        match self {
            Self::Claude => AgentChatProvider::Claude,
            Self::Codex => AgentChatProvider::Codex,
            Self::Claurst => AgentChatProvider::Claurst,
        }
    }

    #[must_use]
    pub const fn provider_id(self) -> &'static str {
        self.provider().contained_contract().provider_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_round_trips_through_its_gateway_and_identity() {
        for provider in [
            AgentChatProvider::Claude,
            AgentChatProvider::Codex,
            AgentChatProvider::Claurst,
        ] {
            let contract = provider.contained_contract();
            assert_eq!(contract.gateway_api.provider(), provider);
            assert_eq!(contract.gateway_api.provider_id(), contract.provider_id);
            assert_eq!(
                AgentChatProvider::from_contained_id(contract.provider_id),
                Some(provider)
            );
            assert!(!contract.credential_contracts.is_empty());
        }
    }

    #[test]
    fn an_unregistered_provider_identity_has_no_contract() {
        for id in ["", "CLAUDE", "gemini", "claude "] {
            assert_eq!(AgentChatProvider::from_contained_id(id), None);
        }
    }
}
