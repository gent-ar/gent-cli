//! Deterministic, Gent-owned storage planning for curated local GGUF models.
//!
//! This module deliberately does not perform HTTP. A downloader can consume a
//! [`LocalModelDownloadPlan`] and report bytes written through [`ModelInstallState`]
//! without gaining authority to choose a model, URL, or destination.

use crate::{
    local_model_catalog::{LocalModelCatalog, LocalModelRecord},
    local_model_integrity::matches_sha256,
};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct LocalModelProvisioner {
    data_dir: PathBuf,
    catalog: LocalModelCatalog,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalModelDownloadPlan {
    pub(crate) model_id: String,
    pub(crate) source_url: String,
    pub(crate) expected_bytes: u64,
    pub(crate) expected_sha256: String,
    pub(crate) destination: PathBuf,
    pub(crate) partial_destination: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModelInstallState {
    NotInstalled,
    Downloading { downloaded_bytes: u64 },
    Ready { path: PathBuf },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum LocalModelProvisioningError {
    #[error("unknown curated local model `{0}`")]
    UnknownModel(String),
    #[error("curated local model `{0}` has an unsafe storage path")]
    UnsafeStoragePath(String),
    #[error("local model file `{0}` is not a regular file")]
    NotRegularFile(PathBuf),
    #[error("local model file `{path}` is {actual_bytes} bytes but expected {expected_bytes}")]
    UnexpectedFileSize {
        path: PathBuf,
        actual_bytes: u64,
        expected_bytes: u64,
    },
    #[error("local model file `{path}` does not match the curated SHA-256")]
    UnexpectedFileDigest { path: PathBuf },
    #[error("could not inspect local model storage: {0}")]
    Io(String),
}

impl LocalModelProvisioner {
    /// `data_dir` must be the daemon's already-authorized, Gent-owned data directory.
    #[must_use]
    pub(crate) fn new(data_dir: impl Into<PathBuf>, catalog: LocalModelCatalog) -> Self {
        Self {
            data_dir: data_dir.into(),
            catalog,
        }
    }

    pub(crate) fn plan(
        &self,
        model_id: &str,
    ) -> Result<LocalModelDownloadPlan, LocalModelProvisioningError> {
        let model = self
            .catalog
            .model(model_id)
            .ok_or_else(|| LocalModelProvisioningError::UnknownModel(model_id.to_owned()))?;
        let directory = self.model_directory(model)?;
        let destination = directory.join(&model.local_filename);
        let partial_destination = directory.join(format!("{}.part", model.local_filename));
        self.validate_contained_path(model, &destination)?;
        self.validate_contained_path(model, &partial_destination)?;
        Ok(LocalModelDownloadPlan {
            model_id: model.id.clone(),
            source_url: model.huggingface_url.clone(),
            expected_bytes: model.size_bytes,
            expected_sha256: model.sha256.clone(),
            destination,
            partial_destination,
        })
    }

    /// Creates only the deterministic parent directory for a previously approved plan.
    pub(crate) fn ensure_storage(
        &self,
        plan: &LocalModelDownloadPlan,
    ) -> Result<(), LocalModelProvisioningError> {
        let model = self.model_for_plan(plan)?;
        let directory = self.model_directory(model)?;
        fs::create_dir_all(directory).map_err(|error| io_error(&error))
    }

    pub(crate) fn state(
        &self,
        model_id: &str,
    ) -> Result<ModelInstallState, LocalModelProvisioningError> {
        let plan = self.plan(model_id)?;
        if let Some(size) = regular_file_size(&plan.destination)? {
            if size == plan.expected_bytes {
                if !matches_sha256(&plan.destination, &plan.expected_sha256)
                    .map_err(|error| io_error(&error))?
                {
                    return Err(LocalModelProvisioningError::UnexpectedFileDigest {
                        path: plan.destination,
                    });
                }
                return Ok(ModelInstallState::Ready {
                    path: plan.destination,
                });
            }
            return Err(LocalModelProvisioningError::UnexpectedFileSize {
                path: plan.destination,
                actual_bytes: size,
                expected_bytes: plan.expected_bytes,
            });
        }
        match regular_file_size(&plan.partial_destination)? {
            Some(downloaded_bytes) if downloaded_bytes <= plan.expected_bytes => {
                Ok(ModelInstallState::Downloading { downloaded_bytes })
            }
            Some(actual_bytes) => Err(LocalModelProvisioningError::UnexpectedFileSize {
                path: plan.partial_destination,
                actual_bytes,
                expected_bytes: plan.expected_bytes,
            }),
            None => Ok(ModelInstallState::NotInstalled),
        }
    }

    pub(crate) fn model(
        &self,
        model_id: &str,
    ) -> Result<&LocalModelRecord, LocalModelProvisioningError> {
        self.catalog
            .model(model_id)
            .ok_or_else(|| LocalModelProvisioningError::UnknownModel(model_id.to_owned()))
    }

    #[must_use]
    pub(crate) fn catalogue(&self) -> &LocalModelCatalog {
        &self.catalog
    }

    fn model_for_plan(
        &self,
        plan: &LocalModelDownloadPlan,
    ) -> Result<&LocalModelRecord, LocalModelProvisioningError> {
        let model = self
            .catalog
            .model(&plan.model_id)
            .ok_or_else(|| LocalModelProvisioningError::UnknownModel(plan.model_id.clone()))?;
        if self.plan(&plan.model_id)? != *plan {
            return Err(LocalModelProvisioningError::UnsafeStoragePath(
                plan.model_id.clone(),
            ));
        }
        Ok(model)
    }

    fn model_directory(
        &self,
        model: &LocalModelRecord,
    ) -> Result<PathBuf, LocalModelProvisioningError> {
        if !single_normal_component(&model.id) || !single_normal_component(&model.local_filename) {
            return Err(LocalModelProvisioningError::UnsafeStoragePath(
                model.id.clone(),
            ));
        }
        Ok(self.data_dir.join("models").join(&model.id))
    }

    fn validate_contained_path(
        &self,
        model: &LocalModelRecord,
        path: &Path,
    ) -> Result<(), LocalModelProvisioningError> {
        let root = self.data_dir.join("models").join(&model.id);
        if path.parent() != Some(root.as_path()) || !path.starts_with(&self.data_dir) {
            return Err(LocalModelProvisioningError::UnsafeStoragePath(
                model.id.clone(),
            ));
        }
        Ok(())
    }
}

fn single_normal_component(value: &str) -> bool {
    matches!(
        Path::new(value).components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(_)]
    )
}

fn regular_file_size(path: &Path) -> Result<Option<u64>, LocalModelProvisioningError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            LocalModelProvisioningError::NotRegularFile(path.to_path_buf()),
        ),
        Ok(metadata) => Ok(Some(metadata.len())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(&error)),
    }
}

fn io_error(error: &std::io::Error) -> LocalModelProvisioningError {
    LocalModelProvisioningError::Io(error.to_string())
}

#[cfg(test)]
#[path = "local_model_provisioning_tests.rs"]
mod tests;
