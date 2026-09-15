use gent_types::AgentChatProvider;

use super::defaults::ModelCatalogDefaults;
use crate::{
    local_model_events::failure_text,
    local_model_jobs::{DownloadHolder, DownloadStart},
    standalone_authority_composition::StandaloneClaurstModels,
};

#[derive(Clone)]
pub(crate) struct LocalModelDownloads {
    models: StandaloneClaurstModels,
    defaults: ModelCatalogDefaults,
}

impl LocalModelDownloads {
    pub(crate) fn new(models: StandaloneClaurstModels, defaults: ModelCatalogDefaults) -> Self {
        Self { models, defaults }
    }

    pub(crate) fn start_default_when_missing(&self) -> Result<bool, String> {
        let selection = self.defaults.selection();
        if selection.provider != AgentChatProvider::Claurst
            || self.defaults.declined(&selection.model)
        {
            return Ok(false);
        }
        self.begin(&selection.model)
    }

    pub(crate) fn start(&self, model_id: &str) -> Result<bool, String> {
        self.defaults.accept(model_id)?;
        self.begin(model_id)
    }

    pub(crate) fn cancel(&self, model_id: &str) -> Result<bool, String> {
        let canceled = self
            .models
            .downloads
            .cancel(model_id)
            .map_err(|failure| failure_text(failure).to_owned())?;
        self.defaults.decline(model_id)?;
        Ok(canceled)
    }

    fn begin(&self, model_id: &str) -> Result<bool, String> {
        match self
            .models
            .downloads
            .start(model_id, DownloadHolder::Background)
        {
            Ok(DownloadStart::Ready { .. }) => Ok(false),
            Ok(DownloadStart::Downloading(_)) => Ok(true),
            Err(failure) => Err(failure_text(failure).to_owned()),
        }
    }
}

#[cfg(test)]
#[path = "model_catalog_downloads_tests.rs"]
pub(crate) mod tests;
