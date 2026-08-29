//! Curated, read-only local GGUF model records for the Claurst adapter.

use serde::Deserialize;
use std::collections::BTreeSet;

const SHIPPED_CATALOGUE: &str = include_str!("../models.json");

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalModelCatalog {
    models: Vec<LocalModelRecord>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalModelRecord {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) huggingface_url: String,
    pub(crate) local_filename: String,
    pub(crate) provider_model_id: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
    #[serde(default)]
    pub(crate) chat_template_file: Option<String>,
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
    models: Vec<LocalModelRecord>,
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
            models: document.models,
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

fn validate(model: &LocalModelRecord) -> Result<(), LocalModelCatalogError> {
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
mod tests {
    use super::{LocalModelCatalog, LocalModelCatalogError};

    #[test]
    fn shipped_catalogue_is_strict_and_queryable() {
        let catalogue = LocalModelCatalog::shipped().unwrap();
        assert_eq!(catalogue.models().len(), 3);
        assert!(
            catalogue
                .model(gent_protocol::DEFAULT_LOCAL_MODEL_ID)
                .is_some()
        );
        assert_eq!(
            catalogue.model("qwen3-8b-q4-k-m").unwrap().size_bytes,
            5_027_783_488
        );
        assert_eq!(
            catalogue.model("qwen3-1-7b-q4-k-m").unwrap().size_bytes,
            1_282_439_264
        );
        assert_eq!(
            catalogue
                .model("hermes-3-llama-3-1-8b-q4-k-m")
                .unwrap()
                .size_bytes,
            4_920_733_824
        );
    }

    #[test]
    fn rejects_unknown_fields_and_unsafe_downloads() {
        let unknown = r#"{"models":[{"id":"model","label":"Model","huggingface_url":"https://huggingface.co/a/b/resolve/0123456789abcdef0123456789abcdef01234567/a.gguf","local_filename":"a.gguf","provider_model_id":"model","size_bytes":1,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","extra":true}]}"#;
        assert_eq!(
            LocalModelCatalog::from_json(unknown),
            Err(LocalModelCatalogError::Malformed)
        );
        let unsafe_url = unknown
            .replace(",\"extra\":true", "")
            .replace("https://huggingface.co", "http://example.test");
        assert_eq!(
            LocalModelCatalog::from_json(&unsafe_url),
            Err(LocalModelCatalogError::InvalidRecord("model".into()))
        );
    }

    #[test]
    fn rejects_duplicate_ids_and_unsafe_filenames() {
        let duplicate = r#"{"models":[{"id":"model","label":"Model","huggingface_url":"https://huggingface.co/a/b/resolve/0123456789abcdef0123456789abcdef01234567/a.gguf","local_filename":"a.gguf","provider_model_id":"model","size_bytes":1,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"id":"model","label":"Model Two","huggingface_url":"https://huggingface.co/a/b/resolve/0123456789abcdef0123456789abcdef01234567/b.gguf","local_filename":"b.gguf","provider_model_id":"model-two","size_bytes":1,"sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]}"#;
        assert_eq!(
            LocalModelCatalog::from_json(duplicate),
            Err(LocalModelCatalogError::Duplicate("model".into()))
        );
        let unsafe_filename = duplicate
            .replace(
                r#""id":"model","label":"Model Two""#,
                r#""id":"model-two","label":"Model Two""#,
            )
            .replace("b.gguf", "../b.gguf");
        assert_eq!(
            LocalModelCatalog::from_json(&unsafe_filename),
            Err(LocalModelCatalogError::InvalidRecord("model-two".into()))
        );
    }
}
