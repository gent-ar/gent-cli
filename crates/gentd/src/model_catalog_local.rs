use std::time::Duration;

use gent_protocol::{
    DEFAULT_LOCAL_MODEL_ID,
    model_catalog::{CatalogModel, LocalModelAvailability, ProviderAvailability},
};
use gent_types::{AgentChatEffort, AgentChatProvider};

use super::service::{ModelCatalogSource, ProviderListing};
use crate::{
    provider_launch_budget::ProviderLaunchError,
    standalone_authority_composition::StandaloneClaurstModels,
};

const LOCAL_EFFORTS: [AgentChatEffort; 6] = [
    AgentChatEffort::Low,
    AgentChatEffort::Medium,
    AgentChatEffort::High,
    AgentChatEffort::XHigh,
    AgentChatEffort::Max,
    AgentChatEffort::Ultra,
];

pub(crate) struct LocalModelSource {
    pub(crate) models: StandaloneClaurstModels,
}

impl ModelCatalogSource for LocalModelSource {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Claurst
    }

    fn label(&self) -> &'static str {
        "Gent"
    }

    fn ttl(&self) -> Duration {
        Duration::ZERO
    }

    fn load(&self) -> Result<ProviderListing, ProviderLaunchError> {
        let models = self
            .models
            .catalogue()
            .into_iter()
            .map(|descriptor| {
                let install = self
                    .models
                    .install_state(&descriptor.id)
                    .map_err(|error| error.to_string())?;
                Ok(CatalogModel {
                    is_default: descriptor.id == DEFAULT_LOCAL_MODEL_ID,
                    id: descriptor.id,
                    label: descriptor.label,
                    description: None,
                    efforts: LOCAL_EFFORTS.to_vec(),
                    default_effort: Some(AgentChatEffort::Medium),
                    local: Some(LocalModelAvailability {
                        size_bytes: descriptor.size_bytes,
                        install,
                    }),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(ProviderListing {
            availability: ProviderAvailability::Ready,
            models,
        })
    }
}
