impl RuntimeFacade {
fn local_model_frame(&self, frame: LocalModelFrame) -> Result<LocalModelFrame, String> {
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
            state: models.install_state(&model_id).map_err(|error| error.to_string())?,
            model_id,
        }),
        LocalModelFrame::Download { .. } => Err("download requests are stream-only".into()),
        _ => Err("local-model response frames are server-only".into()),
    }
}

fn start_local_model_download(
    &self,
    model_id: &str,
) -> Result<crate::standalone_authority_composition::LocalModelDownloadStart, String> {
    self.local_models
        .as_ref()
        .ok_or_else(|| "local models are unavailable for this runtime".to_owned())?
        .begin_download(model_id)
        .map_err(|error| error.to_string())
}

fn complete_local_model_download(&self, model_id: &str) {
    if let Some(models) = &self.local_models {
        models.finish_download(model_id);
    }
}

fn publish_local_model_frame(&self, frame: LocalModelFrame) -> Result<(), String> {
    let host_epoch = self
        .coordinator
        .status()
        .map_err(|error| error.to_string())?
        .host_epoch;
    crate::local_model_events::publish(&self.local_model_events, host_epoch, frame)
}
}
