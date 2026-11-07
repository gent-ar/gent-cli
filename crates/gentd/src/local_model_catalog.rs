use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

const SHIPPED_CATALOGUE: &str = include_str!("../models.json");

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalModelCatalog {
    models: Vec<LocalModelRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalModelRecord {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) provider_model_id: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
    pub(crate) context_tokens: u32,
    pub(crate) runtime_memory_bytes: u64,
    pub(crate) agent_profile: LocalAgentProfile,
    pub(crate) chat_template_file: Option<String>,
    pub(crate) source: LocalModelSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LocalModelSource {
    Curated {
        huggingface_url: String,
        local_filename: String,
    },
    UserFile {
        path: PathBuf,
        modified_unix_nanos: u128,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CuratedModelRecord {
    id: String,
    label: String,
    huggingface_url: String,
    local_filename: String,
    provider_model_id: String,
    size_bytes: u64,
    sha256: String,
    context_tokens: u32,
    runtime_memory_bytes: u64,
    agent_profile: LocalAgentProfile,
    #[serde(default)]
    chat_template_file: Option<String>,
}

impl From<CuratedModelRecord> for LocalModelRecord {
    fn from(record: CuratedModelRecord) -> Self {
        Self {
            id: record.id,
            label: record.label,
            provider_model_id: record.provider_model_id,
            size_bytes: record.size_bytes,
            sha256: record.sha256,
            context_tokens: record.context_tokens,
            runtime_memory_bytes: record.runtime_memory_bytes,
            agent_profile: record.agent_profile,
            chat_template_file: record.chat_template_file,
            source: LocalModelSource::Curated {
                huggingface_url: record.huggingface_url,
                local_filename: record.local_filename,
            },
        }
    }
}

impl LocalModelRecord {
    pub(crate) fn curated(&self) -> Option<(&str, &str)> {
        match &self.source {
            LocalModelSource::Curated {
                huggingface_url,
                local_filename,
            } => Some((huggingface_url, local_filename)),
            LocalModelSource::UserFile { .. } => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LocalAgentProfile {
    Compact,
    Full,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum LocalModelCatalogError {
    #[error("local model catalogue is malformed")]
    Malformed,
    #[error("local model catalogue has invalid record `{0}`")]
    InvalidRecord(String),
    #[error("local model catalogue repeats model `{0}`")]
    Duplicate(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogueDocument {
    models: Vec<CuratedModelRecord>,
}

impl LocalModelCatalog {
    pub(crate) fn shipped() -> Result<Self, LocalModelCatalogError> {
        Self::from_json(SHIPPED_CATALOGUE)
    }

    pub(crate) fn from_json(source: &str) -> Result<Self, LocalModelCatalogError> {
        let document = serde_json::from_str::<CatalogueDocument>(source)
            .map_err(|_| LocalModelCatalogError::Malformed)?;
        if document.models.is_empty() {
            return Err(LocalModelCatalogError::InvalidRecord("models".into()));
        }
        let mut ids = BTreeSet::new();
        let mut filenames = BTreeSet::new();
        for model in &document.models {
            validate(model)?;
            if !ids.insert(model.id.clone()) || !filenames.insert(model.local_filename.clone()) {
                return Err(LocalModelCatalogError::Duplicate(model.id.clone()));
            }
        }
        Ok(Self {
            models: document.models.into_iter().map(Into::into).collect(),
        })
    }

    #[must_use]
    pub(crate) fn models(&self) -> &[LocalModelRecord] {
        &self.models
    }

    #[must_use]
    pub(crate) fn model(&self, id: &str) -> Option<&LocalModelRecord> {
        self.models.iter().find(|model| model.id == id)
    }
}

fn validate(model: &CuratedModelRecord) -> Result<(), LocalModelCatalogError> {
    let identifier = |value: &str| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    if !identifier(&model.id)
        || !identifier(&model.provider_model_id)
        || model.label.trim().is_empty()
        || model.label.len() > 160
        || model.size_bytes == 0
        || !(4_096..=131_072).contains(&model.context_tokens)
        || model.runtime_memory_bytes <= model.size_bytes
        || !sha256(&model.sha256)
        || std::path::Path::new(&model.local_filename)
            .extension()
            .is_none_or(|extension| extension != "gguf")
        || model.local_filename.len() > 160
        || model.local_filename.contains(['/', '\\'])
        || model.chat_template_file.as_deref().is_some_and(|file| {
            file.is_empty()
                || file.len() > 160
                || file.contains(['/', '\\'])
                || !std::path::Path::new(file)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("jinja"))
        })
        || !is_pinned_huggingface_gguf(&model.huggingface_url)
    {
        return Err(LocalModelCatalogError::InvalidRecord(model.id.clone()));
    }
    Ok(())
}

fn sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_pinned_huggingface_gguf(url: &str) -> bool {
    let Some((repository, revision_and_file)) = url
        .strip_prefix("https://huggingface.co/")
        .and_then(|path| path.split_once("/resolve/"))
    else {
        return false;
    };
    let Some((revision, filename)) = revision_and_file.split_once('/') else {
        return false;
    };
    !repository.is_empty()
        && revision.len() == 40
        && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        && std::path::Path::new(filename)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
        && !filename.contains(['/', '?', '#'])
}

#[cfg(test)]
#[path = "local_model_catalog_tests.rs"]
mod tests;
