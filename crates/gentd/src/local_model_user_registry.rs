use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use gent_protocol::model_catalog::LocalModelFileRejection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::user_file::{self, UserModelFile};
use crate::local_model_catalog::{LocalAgentProfile, LocalModelRecord, LocalModelSource};

const REGISTRY_FILE: &str = "user-models.json";
const SCHEMA: u32 = 1;
const MAX_MODELS: usize = 64;
const MAX_SLUG_BYTES: usize = 48;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UserModelEntry {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) path: PathBuf,
    pub(crate) size_bytes: u64,
    pub(crate) modified_unix_nanos: u128,
    pub(crate) sha256: String,
    pub(crate) context_tokens: u32,
    pub(crate) runtime_memory_bytes: u64,
    pub(crate) agent_profile: LocalAgentProfile,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistryDocument {
    schema: u32,
    models: Vec<UserModelEntry>,
}

#[derive(Clone, Debug)]
pub(crate) struct UserModelRegistry {
    path: PathBuf,
    writes: Arc<Mutex<()>>,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum UserModelError {
    #[error("the local model file was rejected: {}", .0.code())]
    Rejected(LocalModelFileRejection),
    #[error("{0}")]
    Storage(String),
}

impl super::LocalModelProvisioner {
    pub(crate) fn add_user_model(&self, path: &str) -> Result<String, UserModelError> {
        let file = user_file::inspect(path).map_err(UserModelError::Rejected)?;
        let unreadable = UserModelError::Rejected(LocalModelFileRejection::Unreadable);
        let sha256 = crate::local_model_integrity::file_sha256(&file.path)
            .map_err(|_| unreadable.clone())?;
        if user_file::identity(&file.path) != Some((file.size_bytes, file.modified_unix_nanos)) {
            return Err(unreadable);
        }
        let entry = UserModelEntry::new(file, sha256);
        let id = entry.id.clone();
        self.user_models
            .add(entry)
            .map_err(UserModelError::Storage)?;
        Ok(id)
    }

    pub(crate) fn remove_user_model(&self, model_id: &str) -> Result<(), UserModelError> {
        if self
            .user_models
            .remove(model_id)
            .map_err(UserModelError::Storage)?
        {
            Ok(())
        } else {
            Err(UserModelError::Rejected(
                LocalModelFileRejection::NotRemovable,
            ))
        }
    }
}

impl UserModelEntry {
    pub(crate) fn new(file: UserModelFile, sha256: String) -> Self {
        Self {
            id: model_id(&file.label, &file.path),
            label: file.label,
            path: file.path,
            size_bytes: file.size_bytes,
            modified_unix_nanos: file.modified_unix_nanos,
            sha256,
            context_tokens: file.context_tokens,
            runtime_memory_bytes: file.runtime_memory_bytes,
            agent_profile: file.agent_profile,
        }
    }

    pub(crate) fn record(&self) -> LocalModelRecord {
        LocalModelRecord {
            id: self.id.clone(),
            label: self.label.clone(),
            provider_model_id: self.id.clone(),
            size_bytes: self.size_bytes,
            sha256: self.sha256.clone(),
            context_tokens: self.context_tokens,
            runtime_memory_bytes: self.runtime_memory_bytes,
            agent_profile: self.agent_profile,
            chat_template_file: None,
            source: LocalModelSource::UserFile {
                path: self.path.clone(),
                modified_unix_nanos: self.modified_unix_nanos,
            },
        }
    }

    fn valid(&self) -> bool {
        self.id.starts_with("user-")
            && self.id.len() <= 128
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !self.label.trim().is_empty()
            && self.label.len() <= 160
            && self.path.is_absolute()
            && self.size_bytes > 0
            && self.sha256.len() == 64
            && self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            && (4_096..=32_768).contains(&self.context_tokens)
            && self.runtime_memory_bytes > self.size_bytes
    }
}

impl UserModelRegistry {
    pub(crate) fn open(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(REGISTRY_FILE),
            writes: Arc::default(),
        }
    }

    pub(crate) fn entries(&self) -> Result<Vec<UserModelEntry>, String> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("could not read the user model registry: {error}")),
        };
        let document = serde_json::from_slice::<RegistryDocument>(&bytes)
            .map_err(|_| "the user model registry is malformed".to_owned())?;
        let mut ids = std::collections::BTreeSet::new();
        if document.schema != SCHEMA
            || document.models.len() > MAX_MODELS
            || !document
                .models
                .iter()
                .all(|entry| entry.valid() && ids.insert(entry.id.clone()))
        {
            return Err("the user model registry is malformed".into());
        }
        Ok(document.models)
    }

    pub(crate) fn add(&self, entry: UserModelEntry) -> Result<(), String> {
        self.update(|models| {
            models.retain(|existing| existing.path != entry.path && existing.id != entry.id);
            if models.len() >= MAX_MODELS {
                return Err("the user model registry is full".into());
            }
            models.push(entry);
            Ok(())
        })
    }

    pub(crate) fn remove(&self, model_id: &str) -> Result<bool, String> {
        let mut removed = false;
        self.update(|models| {
            let before = models.len();
            models.retain(|entry| entry.id != model_id);
            removed = models.len() != before;
            Ok(())
        })?;
        Ok(removed)
    }

    fn update(
        &self,
        change: impl FnOnce(&mut Vec<UserModelEntry>) -> Result<(), String>,
    ) -> Result<(), String> {
        let _write = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        let mut models = self.entries()?;
        change(&mut models)?;
        let encoded = serde_json::to_vec_pretty(&RegistryDocument {
            schema: SCHEMA,
            models,
        })
        .map_err(|error| error.to_string())?;
        let staged = self.path.with_extension("json.tmp");
        fs::write(&staged, encoded)
            .and_then(|()| fs::rename(&staged, &self.path))
            .map_err(|error| format!("could not save the user model registry: {error}"))
    }
}

pub(crate) fn model_id(label: &str, canonical_path: &Path) -> String {
    let mut slug = String::new();
    for character in label.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= MAX_SLUG_BYTES {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    let digest = hex::encode(Sha256::digest(canonical_path.to_string_lossy().as_bytes()));
    if slug.is_empty() {
        format!("user-{}", &digest[..8])
    } else {
        format!("user-{slug}-{}", &digest[..8])
    }
}

#[cfg(test)]
#[path = "local_model_user_registry_tests.rs"]
mod tests;
