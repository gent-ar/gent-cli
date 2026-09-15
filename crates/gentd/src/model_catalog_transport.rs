use gent_protocol::{
    model_catalog::{MODEL_CATALOG_CAPABILITY, ModelCatalogFrame, ModelCatalogSelection},
    write_json_frame,
};
use std::sync::Arc;

use gent_types::{AgentChatSelection, CapabilitySet};
use serde_json::Value;
use tokio::io::AsyncWrite;

use super::{downloads::LocalModelDownloads, service::ModelCatalogService};
use crate::{api::RuntimeApi, transport::write_error};

pub(crate) trait ModelCatalogPort: Send + Sync {
    fn exchange(&self, frame: ModelCatalogFrame) -> Result<ModelCatalogFrame, String>;
    fn default_selection(&self) -> AgentChatSelection;
    fn remember(&self, selection: &AgentChatSelection);
    fn validate(
        &self,
        selection: &AgentChatSelection,
    ) -> Result<(), crate::agent_chat_intent_error::AgentChatIntentError>;
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
    fn exchange(&self, frame: ModelCatalogFrame) -> Result<ModelCatalogFrame, String> {
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
            ModelCatalogFrame::ModelCatalog { .. } => {
                return Err("model-catalog response frames are server-only".into());
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

    fn validate(
        &self,
        selection: &AgentChatSelection,
    ) -> Result<(), crate::agent_chat_intent_error::AgentChatIntentError> {
        self.service.validate(selection)
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
    match port.exchange(frame) {
        Ok(reply) if reply.validate().is_ok() => write_json_frame(stream, &reply).await?,
        Ok(_) => {
            write_error(
                stream,
                "invalidModelCatalog",
                "model catalog runtime returned an invalid catalog",
            )
            .await?;
        }
        Err(message) => write_error(stream, "modelCatalogRejected", &message).await?,
    }
    Ok(true)
}

#[cfg(test)]
#[path = "model_catalog_transport_tests.rs"]
mod tests;
