use std::{sync::Arc, time::Duration};

use gent_protocol::model_catalog::CatalogModel;
use gent_types::{AgentChatProvider, ProviderAuthProvider};
use serde_json::Value;

use super::{
    claude_initialize::ClaudeInitialize,
    service::{
        ModelCatalogSource, PROVIDER_DEFAULT_MODEL, ProviderListing, effort,
        provider_default_model, public_availability,
    },
};
use crate::{provider_auth_api::ProviderAuthPort, provider_launch_budget::ProviderLaunchError};

pub(crate) struct ClaudeModelSource {
    pub(crate) initialize: Arc<ClaudeInitialize>,
    pub(crate) auth: Arc<dyn ProviderAuthPort>,
}

impl ModelCatalogSource for ClaudeModelSource {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Claude
    }

    fn label(&self) -> &'static str {
        "Claude"
    }

    fn ttl(&self) -> Duration {
        Duration::from_secs(600)
    }

    fn revision(&self) -> Option<String> {
        self.initialize.revision()
    }

    fn load(&self) -> Result<ProviderListing, ProviderLaunchError> {
        let availability = public_availability(self.auth.as_ref(), ProviderAuthProvider::Claude)?;
        if !self.initialize.installed() {
            return Ok(ProviderListing {
                availability,
                models: vec![provider_default_model()],
            });
        }
        let response = self.initialize.load(None, true)?;
        let Some(entries) = response["models"].as_array() else {
            return Err(ProviderLaunchError::Failed(
                "claude did not report its models".into(),
            ));
        };
        Ok(ProviderListing {
            availability,
            models: entries.iter().filter_map(claude_model).collect(),
        })
    }
}

fn claude_model(entry: &Value) -> Option<CatalogModel> {
    let id = entry.get("value")?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }
    let efforts = entry
        .get("supportedEffortLevels")
        .and_then(Value::as_array)
        .map(|levels| {
            levels
                .iter()
                .filter_map(Value::as_str)
                .filter_map(effort)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(CatalogModel {
        id: id.into(),
        label: entry
            .get("displayName")
            .and_then(Value::as_str)
            .filter(|label| !label.trim().is_empty())
            .unwrap_or(id)
            .into(),
        description: entry
            .get("description")
            .and_then(Value::as_str)
            .filter(|description| !description.trim().is_empty())
            .map(str::to_owned),
        is_default: id == PROVIDER_DEFAULT_MODEL,
        default_effort: efforts
            .contains(&gent_types::AgentChatEffort::Medium)
            .then_some(gent_types::AgentChatEffort::Medium),
        efforts,
        local: None,
    })
}
