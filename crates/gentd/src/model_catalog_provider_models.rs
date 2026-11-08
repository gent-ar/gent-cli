use std::sync::OnceLock;

use gent_protocol::model_catalog::CatalogModel;
use gent_types::AgentChatProvider;
use serde::Deserialize;

use super::service::{PROVIDER_DEFAULT_MODEL, effort, provider_default_model};

#[path = "model_catalog_last_known.rs"]
mod last_known;
pub(crate) use last_known::{LAST_KNOWN_FILE, LastKnownModels};

const BUNDLED: &str = include_str!("../provider-models.json");

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundled {
    claude: Vec<Curated>,
    codex: Vec<Curated>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Curated {
    id: String,
    label: String,
    efforts: Vec<String>,
    default_effort: Option<String>,
}

pub(crate) fn fallback(provider: AgentChatProvider) -> Vec<CatalogModel> {
    let curated = match provider {
        AgentChatProvider::Claude => &bundled().claude,
        AgentChatProvider::Codex => &bundled().codex,
        AgentChatProvider::Claurst => return Vec::new(),
    };
    with_default(curated.iter().map(model).collect())
}

pub(crate) fn discovered(models: Vec<CatalogModel>) -> Vec<CatalogModel> {
    if models.is_empty() {
        return models;
    }
    with_default(models)
}

pub(crate) fn settle(
    provider: AgentChatProvider,
    models: Vec<CatalogModel>,
    last_known: &LastKnownModels,
) -> Vec<CatalogModel> {
    if provider == AgentChatProvider::Claurst {
        return models;
    }
    if models.is_empty() {
        return last_known
            .get(provider)
            .unwrap_or_else(|| fallback(provider));
    }
    last_known.remember(provider, &models);
    models
}

fn with_default(mut models: Vec<CatalogModel>) -> Vec<CatalogModel> {
    if !models
        .iter()
        .any(|model| model.id == PROVIDER_DEFAULT_MODEL)
    {
        let mut default = provider_default_model();
        default.is_default = !models.iter().any(|model| model.is_default);
        models.push(default);
    }
    models
}

fn bundled() -> &'static Bundled {
    static BUNDLED_MODELS: OnceLock<Bundled> = OnceLock::new();
    BUNDLED_MODELS.get_or_init(|| serde_json::from_str(BUNDLED).unwrap_or_default())
}

fn model(curated: &Curated) -> CatalogModel {
    let efforts = curated
        .efforts
        .iter()
        .filter_map(|value| effort(value))
        .collect::<Vec<_>>();
    CatalogModel {
        id: curated.id.clone(),
        label: curated.label.clone(),
        description: None,
        is_default: false,
        default_effort: curated
            .default_effort
            .as_deref()
            .and_then(effort)
            .filter(|default| efforts.contains(default)),
        efforts,
        local: None,
    }
}

#[cfg(test)]
#[path = "model_catalog_provider_models_tests.rs"]
mod tests;
