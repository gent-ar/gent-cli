use std::{path::PathBuf, sync::Arc};

use gent_protocol::{LocalModelDescriptor, LocalModelInstallState};

use super::StandaloneAuthorityError;
use crate::{
    local_model_catalog::LocalModelCatalog,
    local_model_provisioning::{LocalModelDownloadPlan, LocalModelProvisioner, ModelInstallState},
};

#[derive(Clone, Debug)]
pub(crate) struct StandaloneClaurstModels {
    pub(crate) provisioner: LocalModelProvisioner,
    pub(crate) downloads: crate::local_model_jobs::LocalModelDownloadJobs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StandaloneClaurstModelStatus {
    DownloadRequired {
        plan: LocalModelDownloadPlan,
        downloaded_bytes: u64,
    },
    Ready {
        model_path: PathBuf,
    },
}

impl StandaloneClaurstModels {
    pub(crate) fn from_data_dir(
        data_dir: &std::path::Path,
        events: crate::local_model_events::LocalModelEvents,
    ) -> Result<Self, StandaloneAuthorityError> {
        let catalog =
            LocalModelCatalog::shipped().map_err(|_| StandaloneAuthorityError::ClaurstCatalog)?;
        Ok(Self::with_catalog(data_dir, catalog, events))
    }

    pub(crate) fn with_catalog(
        data_dir: &std::path::Path,
        catalog: LocalModelCatalog,
        events: crate::local_model_events::LocalModelEvents,
    ) -> Self {
        let provisioner = LocalModelProvisioner::new(data_dir, catalog);
        Self {
            downloads: crate::local_model_jobs::LocalModelDownloadJobs::new(
                provisioner.clone(),
                events,
            ),
            provisioner,
        }
    }

    #[must_use]
    pub(crate) fn with_download_transport(
        mut self,
        transport: Arc<dyn crate::local_model_download::ModelDownloadTransport>,
    ) -> Self {
        self.downloads = self.downloads.with_transport(transport);
        self
    }

    pub(crate) fn assess(
        &self,
        model_id: &str,
    ) -> Result<StandaloneClaurstModelStatus, StandaloneAuthorityError> {
        let downloaded_bytes = match self.provisioner.state(model_id) {
            Err(crate::local_model_provisioning::LocalModelProvisioningError::UnknownModel(_)) => {
                return Err(StandaloneAuthorityError::UnknownClaurstModel);
            }
            Ok(ModelInstallState::Ready { path }) => {
                return Ok(StandaloneClaurstModelStatus::Ready { model_path: path });
            }
            Ok(ModelInstallState::NotInstalled) => 0,
            Ok(ModelInstallState::Downloading { downloaded_bytes }) => downloaded_bytes,
            Ok(ModelInstallState::Missing) | Err(_) => {
                return Err(StandaloneAuthorityError::ClaurstStorage);
            }
        };
        Ok(StandaloneClaurstModelStatus::DownloadRequired {
            plan: self
                .provisioner
                .plan(model_id)
                .map_err(|_| StandaloneAuthorityError::UnknownClaurstModel)?,
            downloaded_bytes,
        })
    }

    pub(crate) fn catalogue(&self) -> Vec<LocalModelDescriptor> {
        self.provisioner
            .catalogue()
            .models()
            .iter()
            .map(|model| LocalModelDescriptor {
                id: model.id.clone(),
                label: model.label.clone(),
                size_bytes: model.size_bytes,
            })
            .collect()
    }

    pub(crate) fn install_state(
        &self,
        model_id: &str,
    ) -> Result<LocalModelInstallState, StandaloneAuthorityError> {
        self.downloads
            .install_state(model_id)
            .map_err(|failure| match failure {
                gent_protocol::LocalModelDownloadFailure::UnknownModel => {
                    StandaloneAuthorityError::UnknownClaurstModel
                }
                _ => StandaloneAuthorityError::ClaurstStorage,
            })
    }
}
