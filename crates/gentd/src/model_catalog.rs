#[path = "model_catalog_claude.rs"]
pub(crate) mod claude;
#[path = "model_catalog_claude_initialize.rs"]
pub(crate) mod claude_initialize;
#[path = "model_catalog_codex.rs"]
pub(crate) mod codex;
#[path = "model_catalog_commands.rs"]
pub(crate) mod commands;
#[path = "model_catalog_defaults.rs"]
pub(crate) mod defaults;
#[path = "model_catalog_downloads.rs"]
pub(crate) mod downloads;
#[path = "model_catalog_local.rs"]
pub(crate) mod local;
#[path = "model_catalog_probe.rs"]
pub(crate) mod probe;
#[path = "model_catalog_service.rs"]
pub(crate) mod service;
#[path = "model_catalog_transport.rs"]
pub(crate) mod transport;

use std::{path::Path, sync::Arc, time::Duration};

use gent_protocol::AgentChatIntentFrame;

use crate::{
    provider_auth_api::ProviderAuthPort, standalone_authority_composition::StandaloneClaurstModels,
};

const PROVIDER_PROBE_TIMEOUT: Duration =
    crate::provider_launch_budget::launch_budget(Duration::from_secs(20));

pub(crate) struct StandaloneModelCatalogConfig {
    pub(crate) local_models: StandaloneClaurstModels,
    pub(crate) auth: Arc<dyn ProviderAuthPort>,
    pub(crate) executables: crate::provider_executables::ProviderExecutables,
}

pub(crate) fn compose_standalone(
    data_dir: &Path,
    config: StandaloneModelCatalogConfig,
) -> Arc<dyn transport::ModelCatalogPort> {
    let defaults = defaults::ModelCatalogDefaults::open(data_dir);
    let initialize = claude_initialize::ClaudeInitialize::new(
        config.executables.clone(),
        PROVIDER_PROBE_TIMEOUT,
    );
    let service = service::ModelCatalogService::new(
        vec![
            Arc::new(local::LocalModelSource {
                models: config.local_models.clone(),
            }),
            Arc::new(claude::ClaudeModelSource {
                initialize: Arc::clone(&initialize),
                auth: Arc::clone(&config.auth),
            }),
            Arc::new(codex::CodexModelSource {
                executables: config.executables,
                auth: config.auth,
                timeout: PROVIDER_PROBE_TIMEOUT,
            }),
        ],
        defaults.clone(),
    );
    let downloads = downloads::LocalModelDownloads::new(config.local_models, defaults);
    if let Err(error) = downloads.start_default_when_missing() {
        eprintln!("gentd could not start the default local model download: {error}");
    }
    service.read(false);
    Arc::new(transport::StandaloneModelCatalog {
        service,
        downloads,
        initialize,
    })
}

impl super::RuntimeFacade {
    pub(crate) fn with_model_catalog(
        mut self,
        catalog: Arc<dyn transport::ModelCatalogPort>,
    ) -> Self {
        self.model_catalog = Some(catalog);
        self
    }

    pub(super) fn with_default_selection(
        &self,
        frame: AgentChatIntentFrame,
    ) -> Result<AgentChatIntentFrame, String> {
        match frame {
            AgentChatIntentFrame::CreateConversation {
                request_id,
                receipt_id,
                workspace_path,
                selection: None,
            } => {
                let catalog = self.model_catalog.as_ref().ok_or(
                    "agent-chat conversation creation requires a model selection on this runtime",
                )?;
                Ok(AgentChatIntentFrame::CreateConversation {
                    request_id,
                    receipt_id,
                    workspace_path,
                    selection: Some(catalog.default_selection()),
                })
            }
            frame => Ok(frame),
        }
    }

    pub(super) fn validate_catalog_selection(
        &self,
        frame: &AgentChatIntentFrame,
    ) -> Result<(), crate::agent_chat_intent_error::AgentChatIntentError> {
        let selection = match frame {
            AgentChatIntentFrame::CreateConversation {
                selection: Some(selection),
                ..
            }
            | AgentChatIntentFrame::SwitchSelection { selection, .. } => selection,
            _ => return Ok(()),
        };
        self.model_catalog
            .as_ref()
            .map_or(Ok(()), |catalog| catalog.validate(selection))
    }

    pub(super) fn switched_selection(
        &self,
        frame: &AgentChatIntentFrame,
    ) -> Option<gent_types::AgentChatSelection> {
        match frame {
            AgentChatIntentFrame::SwitchSelection { selection, .. } => Some(selection.clone()),
            _ => None,
        }
    }

    pub(super) fn remember_switched_selection(
        &self,
        selection: Option<gent_types::AgentChatSelection>,
        replies: &[AgentChatIntentFrame],
    ) {
        if let (Some(catalog), Some(selection), [AgentChatIntentFrame::Switched { .. }]) =
            (&self.model_catalog, selection, replies)
        {
            catalog.remember(&selection);
        }
    }
}

#[cfg(all(test, unix))]
#[path = "model_catalog_provider_tests.rs"]
mod provider_tests;

#[cfg(test)]
#[path = "model_catalog_first_run_tests.rs"]
mod first_run_tests;
