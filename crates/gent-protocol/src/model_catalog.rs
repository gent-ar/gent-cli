use gent_types::{AgentChatEffort, AgentChatProvider};
use serde::{Deserialize, Serialize};

use crate::LocalModelInstallState;

pub const MODEL_CATALOG_CAPABILITY: &str = "model-catalog-v1";

const MAX_MODELS_PER_PROVIDER: usize = 128;
const MAX_MODEL_ID_BYTES: usize = 512;
const MAX_TEXT_BYTES: usize = 512;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ModelCatalogFrame {
    ReadModelCatalog {
        request_id: String,
        refresh: bool,
    },
    SetDefaultModel {
        request_id: String,
        selection: ModelCatalogSelection,
    },
    StartModelDownload {
        request_id: String,
        model_id: String,
    },
    CancelModelDownload {
        request_id: String,
        model_id: String,
    },
    ModelCatalog {
        request_id: String,
        catalog: ModelCatalog,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCatalogSelection {
    pub provider: AgentChatProvider,
    pub model: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCatalog {
    pub default_selection: ModelCatalogSelection,
    pub providers: Vec<ProviderModelCatalog>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderModelCatalog {
    pub provider: AgentChatProvider,
    pub label: String,
    pub availability: ProviderAvailability,
    pub listing: ModelListing,
    pub models: Vec<CatalogModel>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", deny_unknown_fields)]
pub enum ProviderAvailability {
    Checking,
    Ready,
    NotInstalled,
    SignedOut,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ModelListing {
    Loading,
    Ready,
    Failed { message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogModel {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub efforts: Vec<AgentChatEffort>,
    pub default_effort: Option<AgentChatEffort>,
    pub local: Option<LocalModelAvailability>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalModelAvailability {
    pub size_bytes: u64,
    pub install: LocalModelInstallState,
}

#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
pub enum ModelCatalogFrameError {
    #[error("invalid model-catalog request identifier")]
    InvalidRequestId,
    #[error("invalid model-catalog model identifier")]
    InvalidModelId,
    #[error("invalid model-catalog provider entry")]
    InvalidProvider,
    #[error("invalid model-catalog model entry")]
    InvalidModel,
}

impl ModelCatalogFrame {
    pub fn validate(&self) -> Result<(), ModelCatalogFrameError> {
        let request_id = match self {
            Self::ReadModelCatalog { request_id, .. }
            | Self::SetDefaultModel { request_id, .. }
            | Self::StartModelDownload { request_id, .. }
            | Self::CancelModelDownload { request_id, .. }
            | Self::ModelCatalog { request_id, .. } => request_id,
        };
        if !bounded_text(request_id, 128) || request_id.trim() != request_id {
            return Err(ModelCatalogFrameError::InvalidRequestId);
        }
        match self {
            Self::SetDefaultModel { selection, .. } if !valid_model_id(&selection.model) => {
                Err(ModelCatalogFrameError::InvalidModelId)
            }
            Self::StartModelDownload { model_id, .. }
            | Self::CancelModelDownload { model_id, .. }
                if !valid_model_id(model_id) =>
            {
                Err(ModelCatalogFrameError::InvalidModelId)
            }
            Self::ModelCatalog { catalog, .. } => catalog.validate(),
            _ => Ok(()),
        }
    }
}

impl ModelCatalog {
    fn validate(&self) -> Result<(), ModelCatalogFrameError> {
        if !valid_model_id(&self.default_selection.model) {
            return Err(ModelCatalogFrameError::InvalidModelId);
        }
        let mut providers = Vec::new();
        for entry in &self.providers {
            if providers.contains(&entry.provider)
                || !bounded_text(&entry.label, MAX_TEXT_BYTES)
                || entry.models.len() > MAX_MODELS_PER_PROVIDER
                || matches!(&entry.listing, ModelListing::Failed { message } if !bounded_text(message, MAX_TEXT_BYTES))
            {
                return Err(ModelCatalogFrameError::InvalidProvider);
            }
            providers.push(entry.provider);
            entry.models.iter().try_for_each(CatalogModel::validate)?;
        }
        Ok(())
    }
}

impl CatalogModel {
    fn validate(&self) -> Result<(), ModelCatalogFrameError> {
        let valid_local = self.local.as_ref().is_none_or(|local| {
            local.size_bytes > 0
                && match local.install {
                    LocalModelInstallState::NotInstalled => true,
                    LocalModelInstallState::Downloading {
                        downloaded_bytes,
                        total_bytes,
                    } => total_bytes > 0 && downloaded_bytes <= total_bytes,
                    LocalModelInstallState::Ready { size_bytes } => size_bytes > 0,
                }
        });
        if !valid_model_id(&self.id)
            || !bounded_text(&self.label, MAX_TEXT_BYTES)
            || self
                .description
                .as_deref()
                .is_some_and(|description| !bounded_text(description, MAX_TEXT_BYTES))
            || self
                .default_effort
                .is_some_and(|effort| !self.efforts.contains(&effort))
            || !valid_local
        {
            return Err(ModelCatalogFrameError::InvalidModel);
        }
        Ok(())
    }
}

fn valid_model_id(value: &str) -> bool {
    bounded_text(value, MAX_MODEL_ID_BYTES) && value.trim() == value
}

fn bounded_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

#[cfg(test)]
#[path = "model_catalog_tests.rs"]
mod tests;
