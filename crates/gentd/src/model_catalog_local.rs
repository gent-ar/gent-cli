use std::time::Duration;

use gent_protocol::{
    DEFAULT_LOCAL_MODEL_ID,
    model_catalog::{CatalogModel, LocalModelAvailability, ProviderAvailability},
};
use gent_types::{AgentChatEffort, AgentChatProvider};

use super::service::{ModelCatalogSource, ProviderListing};
use crate::{
    local_model_catalog::LocalModelSource as RecordSource,
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
            .provisioner
            .models()
            .into_iter()
            .map(|record| {
                let install = self
                    .models
                    .install_state(&record.id)
                    .map_err(|error| error.to_string())?;
                let file_name = match &record.source {
                    RecordSource::Curated { .. } => None,
                    RecordSource::UserFile { path, .. } => path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned()),
                };
                Ok(CatalogModel {
                    is_default: record.id == DEFAULT_LOCAL_MODEL_ID,
                    id: record.id,
                    label: record.label,
                    description: None,
                    efforts: LOCAL_EFFORTS.to_vec(),
                    default_effort: Some(AgentChatEffort::Medium),
                    local: Some(LocalModelAvailability {
                        size_bytes: record.size_bytes,
                        install,
                        removable: file_name.is_some(),
                        file_name,
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
