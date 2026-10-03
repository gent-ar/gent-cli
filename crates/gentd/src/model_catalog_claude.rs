use std::{sync::Arc, time::Duration};

use gent_protocol::model_catalog::CatalogModel;
use gent_types::{AgentChatProvider, ProviderAuthProvider};
use serde_json::Value;

use super::{
    claude_initialize::ClaudeInitialize,
    service::{ModelCatalogSource, ProviderListing, effort, public_availability},
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
                // This is a real Claude selector, not the provider's opaque
                // `default` alias.  It is replaced with the installed CLI's
                // catalog as soon as the provider becomes available.
                models: vec![bootstrap_fable()],
            });
        }
        let response = self.initialize.load(None, true)?;
        let Some(entries) = response["models"].as_array() else {
            return Err(ProviderLaunchError::Failed(
                "claude did not report its models".into(),
            ));
        };
        let mut models = entries
            .iter()
            // `default` is an account policy alias. It has no stable model
            // name or cost, so it must not be exposed as a user model choice.
            .filter_map(claude_model)
            .filter(|model| model.id != "default")
            .collect::<Vec<_>>();
        mark_lowest_cost(&mut models);
        Ok(ProviderListing {
            availability,
            models,
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
        is_default: false,
        default_effort: efforts
            .contains(&gent_types::AgentChatEffort::Medium)
            .then_some(gent_types::AgentChatEffort::Medium),
        efforts,
        local: None,
    })
}

fn bootstrap_fable() -> CatalogModel {
    CatalogModel {
        id: "fable".into(),
        label: "Fable".into(),
        description: Some("Lowest-cost Claude model".into()),
        is_default: true,
        efforts: Vec::new(),
        default_effort: None,
        local: None,
    }
}

fn mark_lowest_cost(models: &mut [CatalogModel]) {
    // Claude's initialize response has no price field. Its public model
    // families provide the only stable cost signal, ordered from least to
    // most expensive. Unknown future models stay available but never replace
    // a known lower-cost family.
    let preferred = models
        .iter()
        .enumerate()
        .min_by_key(|(_, model)| claude_cost_rank(model))
        .map(|(index, _)| index);
    for model in models.iter_mut() {
        model.is_default = false;
    }
    if let Some(index) = preferred {
        models[index].is_default = true;
    }
}

fn claude_cost_rank(model: &CatalogModel) -> u8 {
    let identity = format!("{} {}", model.id, model.label).to_ascii_lowercase();
    if identity.contains("fable") {
        0
    } else if identity.contains("haiku") {
        1
    } else if identity.contains("sonnet") {
        2
    } else if identity.contains("opus") {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{claude_model, mark_lowest_cost};

    #[test]
    fn hides_the_account_default_alias_and_marks_fable_as_the_lowest_cost_choice() {
        let entries = [
            json!({"value": "default", "displayName": "Default (recommended)"}),
            json!({"value": "opus", "displayName": "Opus"}),
            json!({"value": "fable", "displayName": "Fable"}),
            json!({"value": "haiku", "displayName": "Haiku"}),
        ];
        let mut models = entries
            .iter()
            .filter_map(claude_model)
            .filter(|model| model.id != "default")
            .collect::<Vec<_>>();
        mark_lowest_cost(&mut models);

        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["opus", "fable", "haiku"]
        );
        assert_eq!(
            models
                .iter()
                .find(|model| model.is_default)
                .map(|model| model.id.as_str()),
            Some("fable")
        );
    }
}
