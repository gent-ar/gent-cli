//! Deterministic, Gent-owned storage planning for curated local GGUF models.
//!
//! This module deliberately does not perform HTTP. A downloader can consume a
//! [`LocalModelDownloadPlan`] and report bytes written through [`ModelInstallState`]
//! without gaining authority to choose a model, URL, or destination.

use crate::local_model_catalog::{LocalModelCatalog, LocalModelRecord};
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
        fs::create_dir_all(directory).map_err(io_error)
    }

    pub(crate) fn state(
        &self,
        model_id: &str,
    ) -> Result<ModelInstallState, LocalModelProvisioningError> {
        let plan = self.plan(model_id)?;
        if let Some(size) = regular_file_size(&plan.destination)? {
            if size == plan.expected_bytes {
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
        Err(error) => Err(io_error(error)),
    }
}

fn io_error(error: std::io::Error) -> LocalModelProvisioningError {
    LocalModelProvisioningError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{LocalModelProvisioner, ModelInstallState};
    use crate::local_model_catalog::LocalModelCatalog;
    use std::fs;
    use tempfile::tempdir;

    fn provisioner() -> (tempfile::TempDir, LocalModelProvisioner) {
        let directory = tempdir().unwrap();
        let provisioner =
            LocalModelProvisioner::new(directory.path(), LocalModelCatalog::shipped().unwrap());
        (directory, provisioner)
    }

    #[test]
    fn catalog_model_gets_a_deterministic_gent_owned_download_plan() {
        let (directory, provisioner) = provisioner();
        let plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
        assert_eq!(
            plan.destination,
            directory
                .path()
                .join("models/qwen3-1-7b-q4-k-m/qwen3-1-7b-q4-k-m.gguf")
        );
        assert_eq!(
            plan.partial_destination,
            directory
                .path()
                .join("models/qwen3-1-7b-q4-k-m/qwen3-1-7b-q4-k-m.gguf.part")
        );
        assert!(plan.source_url.starts_with("https://huggingface.co/"));
        assert_eq!(
            provisioner.state(&plan.model_id).unwrap(),
            ModelInstallState::NotInstalled
        );
    }

    #[test]
    fn state_reports_resumable_partial_and_exact_completed_file() {
        let (_directory, provisioner) = provisioner();
        let plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
        provisioner.ensure_storage(&plan).unwrap();
        fs::write(&plan.partial_destination, vec![0_u8; 7]).unwrap();
        assert_eq!(
            provisioner.state(&plan.model_id).unwrap(),
            ModelInstallState::Downloading {
                downloaded_bytes: 7
            }
        );
        fs::remove_file(&plan.partial_destination).unwrap();
        fs::File::create(&plan.destination)
            .unwrap()
            .set_len(plan.expected_bytes)
            .unwrap();
        assert_eq!(
            provisioner.state(&plan.model_id).unwrap(),
            ModelInstallState::Ready {
                path: plan.destination
            }
        );
    }

    #[test]
    fn refuses_unknown_and_tampered_download_plans() {
        let (_directory, provisioner) = provisioner();
        assert!(provisioner.plan("../../outside").is_err());
        let mut plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
        plan.destination = std::path::PathBuf::from("/tmp/outside.gguf");
        assert!(provisioner.ensure_storage(&plan).is_err());
    }

    #[test]
    fn rejects_incomplete_final_files() {
        let (_directory, provisioner) = provisioner();
        let plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
        provisioner.ensure_storage(&plan).unwrap();
        fs::write(&plan.destination, [1_u8]).unwrap();
        assert!(provisioner.state(&plan.model_id).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_model_files() {
        let (_directory, provisioner) = provisioner();
        let plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
        provisioner.ensure_storage(&plan).unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), &plan.destination).unwrap();
        assert!(provisioner.state(&plan.model_id).is_err());
    }
}
