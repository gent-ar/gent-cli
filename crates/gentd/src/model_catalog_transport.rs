use gent_protocol::{
    model_catalog::{
        LOCAL_MODEL_FILES_CAPABILITY, MODEL_CATALOG_CAPABILITY, ModelCatalogFrame,
        ModelCatalogSelection,
    },
    write_json_frame,
};
use std::sync::Arc;

use gent_types::{AgentChatProvider, AgentChatSelection, CapabilitySet};
use serde_json::Value;
use tokio::io::AsyncWrite;

use super::{downloads::LocalModelDownloads, service::ModelCatalogService};
use crate::{api::RuntimeApi, local_model_provisioning::UserModelError, transport::write_error};

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ModelCatalogRejection {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl From<String> for ModelCatalogRejection {
    fn from(message: String) -> Self {
        Self {
            code: "modelCatalogRejected",
            message,
        }
    }
}

impl From<UserModelError> for ModelCatalogRejection {
    fn from(error: UserModelError) -> Self {
        match error {
            UserModelError::Rejected(rejection) => Self {
                code: rejection.code(),
                message: error.to_string(),
            },
            UserModelError::Storage(message) => message.into(),
        }
    }
}

pub(crate) trait ModelCatalogPort: Send + Sync {
    fn exchange(
        &self,
        frame: ModelCatalogFrame,
    ) -> Result<ModelCatalogFrame, ModelCatalogRejection>;
    fn default_selection(&self) -> AgentChatSelection;
    fn remember(&self, selection: &AgentChatSelection);
    fn fit(
        &self,
        selection: &AgentChatSelection,
    ) -> Result<AgentChatSelection, crate::agent_chat_intent_error::AgentChatIntentError>;
    fn provider_commands(
        &self,
        _: gent_types::AgentChatProvider,
        _: Option<std::path::PathBuf>,
        _: bool,
    ) -> super::commands::ProviderCommands {
        super::commands::ProviderCommands::none()
    }
}

pub(crate) struct StandaloneModelCatalog {
    pub(crate) service: ModelCatalogService,
    pub(crate) downloads: LocalModelDownloads,
    pub(crate) initialize: Arc<super::claude_initialize::ClaudeInitialize>,
}

impl ModelCatalogPort for StandaloneModelCatalog {
    fn exchange(
        &self,
        frame: ModelCatalogFrame,
    ) -> Result<ModelCatalogFrame, ModelCatalogRejection> {
        let (request_id, catalog) = match frame {
            ModelCatalogFrame::ReadModelCatalog {
                request_id,
                refresh,
            } => (request_id, self.service.read(refresh)),
            ModelCatalogFrame::SetDefaultModel {
                request_id,
                selection,
            } => (request_id, self.service.set_default(selection)?),
            ModelCatalogFrame::StartModelDownload {
                request_id,
                model_id,
            } => {
                self.downloads.start(&model_id)?;
                (request_id, self.service.read(false))
            }
            ModelCatalogFrame::CancelModelDownload {
                request_id,
                model_id,
            } => {
                self.downloads.cancel(&model_id)?;
                (request_id, self.service.read(false))
            }
            ModelCatalogFrame::AddLocalModel { request_id, path } => {
                self.downloads.add_file(&path)?;
                self.service.reload_uncached(AgentChatProvider::Claurst);
                (request_id, self.service.snapshot())
            }
            ModelCatalogFrame::RemoveLocalModel {
                request_id,
                model_id,
            } => {
                self.downloads.remove_file(&model_id)?;
                self.service.reload_uncached(AgentChatProvider::Claurst);
                (request_id, self.service.snapshot())
            }
            ModelCatalogFrame::ModelCatalog { .. } => {
                return Err(String::from("model-catalog response frames are server-only").into());
            }
        };
        Ok(ModelCatalogFrame::ModelCatalog {
            request_id,
            catalog,
        })
    }

    fn default_selection(&self) -> AgentChatSelection {
        self.service.default_agent_selection()
    }

    fn fit(
        &self,
        selection: &AgentChatSelection,
    ) -> Result<AgentChatSelection, crate::agent_chat_intent_error::AgentChatIntentError> {
        self.service.fit(selection)
    }

    fn provider_commands(
        &self,
        provider: gent_types::AgentChatProvider,
        workspace: Option<std::path::PathBuf>,
        refresh: bool,
    ) -> super::commands::ProviderCommands {
        super::commands::provider_commands(&self.initialize, provider, workspace, refresh)
    }

    fn remember(&self, selection: &AgentChatSelection) {
        let _ = self.service.set_default(ModelCatalogSelection {
            provider: selection.provider,
            model: selection.model.clone(),
        });
    }
}

pub(crate) async fn dispatch<S, R>(
    stream: &mut S,
    runtime: &R,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    dispatch_port(stream, runtime.model_catalog_port(), capabilities, raw).await
}

async fn dispatch_port<S>(
    stream: &mut S,
    port: Option<Arc<dyn ModelCatalogPort>>,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
{
    if !capabilities
        .0
        .iter()
        .any(|capability| capability == MODEL_CATALOG_CAPABILITY)
    {
        return Ok(false);
    }
    let Ok(frame) = serde_json::from_value::<ModelCatalogFrame>(raw.clone()) else {
        return Ok(false);
    };
    let local_files = capabilities
        .0
        .iter()
        .any(|capability| capability == LOCAL_MODEL_FILES_CAPABILITY);
    if !local_files
        && matches!(
            frame,
            ModelCatalogFrame::AddLocalModel { .. } | ModelCatalogFrame::RemoveLocalModel { .. }
        )
    {
        return Ok(false);
    }
    if let Err(error) = frame.validate() {
        write_error(stream, "invalidModelCatalog", &error.to_string()).await?;
        return Ok(true);
    }
    let Some(port) = port else {
        write_error(
            stream,
            "modelCatalogUnavailable",
            "the model catalog is unavailable for this runtime",
        )
        .await?;
        return Ok(true);
    };
    match tokio::task::spawn_blocking(move || port.exchange(frame)).await? {
        Ok(reply) if reply.validate().is_ok() => {
            write_json_frame(stream, &visible_to(reply, local_files)).await?;
        }
        Ok(_) => {
            write_error(
                stream,
                "invalidModelCatalog",
                "model catalog runtime returned an invalid catalog",
            )
            .await?;
        }
        Err(rejection) => write_error(stream, rejection.code, &rejection.message).await?,
    }
    Ok(true)
}

fn visible_to(mut reply: ModelCatalogFrame, local_files: bool) -> ModelCatalogFrame {
    if let (false, ModelCatalogFrame::ModelCatalog { catalog, .. }) = (local_files, &mut reply) {
        for provider in &mut catalog.providers {
            provider
                .models
                .retain(|model| !model.local.as_ref().is_some_and(|local| local.removable));
        }
    }
    reply
}

#[cfg(test)]
#[path = "model_catalog_transport_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "model_catalog_user_models_tests.rs"]
pub(crate) mod user_models_tests;
