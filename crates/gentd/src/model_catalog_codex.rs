use std::{sync::Arc, time::Duration};

use gent_drivers::{
    PublicProvider, message_encoding::encode_codex_handshake, supervisor::ProviderProcess,
};
use gent_protocol::model_catalog::CatalogModel;
use gent_types::{AgentChatProvider, ProviderAuthProvider};
use serde_json::{Value, json};

use super::{
    probe::{ProbeStep, ProviderProbe},
    service::{ModelCatalogSource, ProviderListing, effort, public_availability},
};
use crate::{provider_auth_api::ProviderAuthPort, provider_launch_budget::ProviderLaunchError};

const HANDSHAKE_REQUEST_ID: u64 = 1;
const FIRST_PAGE_REQUEST_ID: u64 = 2;
const MAX_PAGES: u64 = 8;

pub(crate) struct CodexModelSource {
    pub(crate) executables: crate::provider_executables::ProviderExecutables,
    pub(crate) auth: Arc<dyn ProviderAuthPort>,
    pub(crate) timeout: Duration,
}

impl ModelCatalogSource for CodexModelSource {
    fn provider(&self) -> AgentChatProvider {
        AgentChatProvider::Codex
    }

    fn label(&self) -> &'static str {
        "Codex"
    }

    fn ttl(&self) -> Duration {
        Duration::from_secs(600)
    }

    fn revision(&self) -> Option<String> {
        self.executables.revision(AgentChatProvider::Codex)
    }

    fn load(&self) -> Result<ProviderListing, ProviderLaunchError> {
        let availability = public_availability(self.auth.as_ref(), ProviderAuthProvider::Codex)?;
        let Some(executable) = self.executables.executable(AgentChatProvider::Codex) else {
            return Ok(ProviderListing {
                availability,
                models: vec![bootstrap_luna()],
            });
        };
        let mut opening =
            encode_codex_handshake(HANDSHAKE_REQUEST_ID).map_err(|error| error.to_string())?;
        opening.push(model_list_request(FIRST_PAGE_REQUEST_ID, None)?);
        let mut listing = ModelListPages::default();
        let models = ProviderProbe {
            provider: PublicProvider::Codex,
            executable,
            arguments: gent_drivers::launch_spec::codex_app_server_arguments(),
            timeout: self.timeout,
            workspace_root: None,
        }
        .exchange(&opening, |value, process| listing.accept(value, process))?;
        Ok(ProviderListing {
            availability,
            models,
        })
    }
}

#[derive(Default)]
pub(crate) struct ModelListPages {
    models: Vec<CatalogModel>,
    page: u64,
}

impl ModelListPages {
    pub(crate) fn accept(
        &mut self,
        frame: &Value,
        process: &dyn ProviderProcess,
    ) -> Result<ProbeStep<Vec<CatalogModel>>, String> {
        let expected = FIRST_PAGE_REQUEST_ID + self.page;
        if frame.get("id").and_then(Value::as_u64) != Some(expected) {
            return Ok(ProbeStep::Continue);
        }
        if let Some(error) = frame.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("model/list failed");
            return Err(format!("codex could not list models: {message}"));
        }
        let result = &frame["result"];
        let entries = result["data"]
            .as_array()
            .ok_or("codex returned an invalid model list")?;
        for model in entries.iter().filter_map(codex_model) {
            if !self.models.iter().any(|known| known.id == model.id) {
                self.models.push(model);
            }
        }
        let cursor = result
            .get("nextCursor")
            .and_then(Value::as_str)
            .filter(|cursor| !cursor.is_empty());
        match cursor {
            Some(cursor) if self.page + 1 < MAX_PAGES => {
                self.page += 1;
                process
                    .write_frame(&model_list_request(expected + 1, Some(cursor))?)
                    .map_err(|error| error.to_string())?;
                Ok(ProbeStep::Continue)
            }
            _ => {
                let mut models = std::mem::take(&mut self.models);
                mark_lowest_cost(&mut models);
                Ok(ProbeStep::Done(models))
            }
        }
    }
}

fn model_list_request(id: u64, cursor: Option<&str>) -> Result<Vec<u8>, String> {
    let mut frame = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "model/list",
        "params": {"cursor": cursor, "includeHidden": false},
    }))
    .map_err(|error| error.to_string())?;
    frame.push(b'\n');
    Ok(frame)
}

fn codex_model(entry: &Value) -> Option<CatalogModel> {
    if entry.get("hidden").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let id = entry
        .get("model")
        .or_else(|| entry.get("id"))?
        .as_str()?
        .trim();
    if id.is_empty() {
        return None;
    }
    let efforts = entry
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| option.get("reasoningEffort").and_then(Value::as_str))
                .filter_map(effort)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let default_effort = entry
        .get("defaultReasoningEffort")
        .and_then(Value::as_str)
        .and_then(effort)
        .filter(|default| efforts.contains(default));
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
        efforts,
        default_effort,
        local: None,
    })
}

fn bootstrap_luna() -> CatalogModel {
    CatalogModel {
        id: "gpt-5.6-luna".into(),
        label: "GPT-5.6-Luna".into(),
        description: Some("Lowest-cost Codex model".into()),
        is_default: true,
        efforts: Vec::new(),
        default_effort: None,
        local: None,
    }
}

fn mark_lowest_cost(models: &mut [CatalogModel]) {
    // Codex does not return pricing in model/list. Luna is the low-cost
    // family; mini is the next-best explicit cost signal for future lists.
    let preferred = models
        .iter()
        .enumerate()
        .min_by_key(|(_, model)| codex_cost_rank(model))
        .map(|(index, _)| index);
    for model in models.iter_mut() {
        model.is_default = false;
    }
    if let Some(index) = preferred {
        models[index].is_default = true;
    }
}

fn codex_cost_rank(model: &CatalogModel) -> u8 {
    let identity = format!("{} {}", model.id, model.label).to_ascii_lowercase();
    if identity.contains("luna") {
        0
    } else if identity.contains("mini") {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{codex_model, mark_lowest_cost};

    #[test]
    fn marks_luna_as_the_lowest_cost_model_instead_of_the_provider_default() {
        let entries = [
            json!({"model": "gpt-6-astra", "displayName": "GPT-6-Astra", "isDefault": true}),
            json!({"model": "gpt-5.6-luna", "displayName": "GPT-5.6-Luna", "isDefault": false}),
        ];
        let mut models = entries.iter().filter_map(codex_model).collect::<Vec<_>>();
        mark_lowest_cost(&mut models);

        assert_eq!(
            models
                .iter()
                .find(|model| model.is_default)
                .map(|model| model.id.as_str()),
            Some("gpt-5.6-luna")
        );
    }
}
