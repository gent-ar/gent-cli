use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[path = "speech_catalog_rules.rs"]
mod rules;
use rules::{label, unique};

pub const SPEECH_CATALOG_CAPABILITY: &str = "speech-catalog-v1";
pub const SPEECH_CATALOG_SCHEMA: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SpeechCatalogFrame {
    ReadSpeechCatalog {
        request_id: String,
    },
    SpeechCatalog {
        request_id: String,
        revision: String,
        catalog: SpeechCatalog,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeechCatalog {
    pub schema: u32,
    pub cloud_providers: Vec<CloudSpeechProvider>,
    pub local_engines: Vec<LocalSpeechEngine>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudSpeechProvider {
    pub id: String,
    pub label: String,
    pub transcription_url: String,
    pub models_url: Option<String>,
    pub auth: SpeechAuthScheme,
    pub api_key_hint: String,
    pub default_model_id: Option<String>,
    pub models: Vec<SpeechModel>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeechAuthScheme {
    Bearer,
    Token,
    XiApiKey,
    Authorization,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeechModel {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalSpeechEngine {
    pub id: String,
    pub label: String,
    pub models: Vec<LocalSpeechModel>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalSpeechModel {
    pub id: String,
    pub size: String,
    pub label: String,
    pub filename: String,
    pub size_bytes: u64,
    pub url: String,
    pub sha256: String,
    pub english_only: bool,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum SpeechCatalogError {
    #[error("speech catalogue is malformed")]
    Malformed,
    #[error("speech catalogue schema {0} is unsupported")]
    UnsupportedSchema(u32),
    #[error("speech catalogue has invalid entry `{0}`")]
    InvalidEntry(String),
    #[error("speech catalogue repeats `{0}`")]
    Duplicate(String),
    #[error("invalid speech-catalog request identifier")]
    InvalidRequestId,
    #[error("speech catalogue revision does not match its content")]
    RevisionMismatch,
}

impl SpeechCatalog {
    pub fn from_json(source: &str) -> Result<Self, SpeechCatalogError> {
        let catalog =
            serde_json::from_str::<Self>(source).map_err(|_| SpeechCatalogError::Malformed)?;
        catalog.validate()?;
        Ok(catalog)
    }

    #[must_use]
    pub fn revision(&self) -> String {
        let canonical = serde_json::to_vec(self).unwrap_or_default();
        hex::encode(Sha256::digest(canonical))
    }

    pub fn validate(&self) -> Result<(), SpeechCatalogError> {
        if self.schema != SPEECH_CATALOG_SCHEMA {
            return Err(SpeechCatalogError::UnsupportedSchema(self.schema));
        }
        if self.cloud_providers.is_empty() && self.local_engines.is_empty() {
            return Err(SpeechCatalogError::InvalidEntry("catalogue".into()));
        }
        let mut sources = Vec::new();
        for provider in &self.cloud_providers {
            provider.validate()?;
            unique(&mut sources, &provider.id)?;
        }
        for engine in &self.local_engines {
            engine.validate()?;
            unique(&mut sources, &engine.id)?;
        }
        Ok(())
    }
}

impl SpeechCatalogFrame {
    pub fn validate(&self) -> Result<(), SpeechCatalogError> {
        let request_id = match self {
            Self::ReadSpeechCatalog { request_id } | Self::SpeechCatalog { request_id, .. } => {
                request_id
            }
        };
        if !label(request_id, 128) || request_id.trim() != request_id {
            return Err(SpeechCatalogError::InvalidRequestId);
        }
        match self {
            Self::ReadSpeechCatalog { .. } => Ok(()),
            Self::SpeechCatalog {
                revision, catalog, ..
            } => {
                catalog.validate()?;
                if *revision == catalog.revision() {
                    Ok(())
                } else {
                    Err(SpeechCatalogError::RevisionMismatch)
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "speech_catalog_tests.rs"]
mod tests;
