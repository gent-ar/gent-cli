use gent_protocol::{LocalModelDownloadFailure, LocalModelFrame};

use super::RuntimeFacade;
use crate::local_model_jobs::{DownloadHolder, DownloadStart};

impl RuntimeFacade {
    pub(super) fn local_model_frame(
        &self,
        frame: LocalModelFrame,
    ) -> Result<LocalModelFrame, String> {
        let models = self
            .local_models
            .as_ref()
            .ok_or_else(|| "local models are unavailable for this runtime".to_owned())?;
        match frame {
            LocalModelFrame::List { request_id } => Ok(LocalModelFrame::Catalogue {
                request_id,
                models: models.catalogue(),
            }),
            LocalModelFrame::Status {
                request_id,
                model_id,
            } => Ok(LocalModelFrame::StatusResult {
                request_id,
                state: models
                    .install_state(&model_id)
                    .map_err(|error| error.to_string())?,
                model_id,
            }),
            LocalModelFrame::Download { .. } => Err("download requests are stream-only".into()),
            _ => Err("local-model response frames are server-only".into()),
        }
    }

    pub(super) fn start_background_model_download(
        &self,
        model_id: &str,
    ) -> Result<DownloadStart, LocalModelDownloadFailure> {
        self.local_models
            .as_ref()
            .ok_or(LocalModelDownloadFailure::StorageUnavailable)?
            .downloads
            .start(model_id, DownloadHolder::Background)
    }
}
