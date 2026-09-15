use gent_protocol::model_catalog::ModelCatalog;
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};

use super::{Mode, Provider};
use crate::model_catalog_cli::catalog_model;

pub(crate) const fn provider(value: Provider) -> AgentChatProvider {
    match value {
        Provider::Claude => AgentChatProvider::Claude,
        Provider::Codex => AgentChatProvider::Codex,
        Provider::Gent => AgentChatProvider::Claurst,
    }
}

pub(crate) const fn provider_name(value: AgentChatProvider) -> &'static str {
    match value {
        AgentChatProvider::Claude => "Claude",
        AgentChatProvider::Codex => "Codex",
        AgentChatProvider::Claurst => "Gent",
    }
}

const EFFORTS: [(AgentChatEffort, &str); 6] = [
    (AgentChatEffort::Low, "low"),
    (AgentChatEffort::Medium, "medium"),
    (AgentChatEffort::High, "high"),
    (AgentChatEffort::XHigh, "xhigh"),
    (AgentChatEffort::Max, "max"),
    (AgentChatEffort::Ultra, "ultra"),
];

pub(crate) fn parse_effort(value: &str) -> Result<AgentChatEffort, String> {
    EFFORTS
        .iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(value))
        .map(|(effort, _)| *effort)
        .ok_or_else(|| format!("`{value}` is not a reasoning effort name"))
}

pub(crate) fn fitted_effort(
    catalog: Option<&ModelCatalog>,
    provider: AgentChatProvider,
    model: &str,
    current: Option<AgentChatEffort>,
) -> AgentChatEffort {
    let listed = catalog.and_then(|catalog| catalog_model(catalog, provider, model));
    match (current, listed) {
        (Some(current), None) => current,
        (Some(current), Some(listed))
            if listed.efforts.is_empty() || listed.efforts.contains(&current) =>
        {
            current
        }
        (_, Some(listed)) => listed
            .default_effort
            .or_else(|| listed.efforts.first().copied())
            .unwrap_or(AgentChatEffort::Medium),
        (None, None) => AgentChatEffort::Medium,
    }
}

pub(crate) const fn mode(value: Mode) -> AgentChatMode {
    match value {
        Mode::Ask => AgentChatMode::Ask,
        Mode::Plan => AgentChatMode::Plan,
        Mode::Agent => AgentChatMode::Agent,
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SelectionRequest {
    pub(crate) provider: Option<Provider>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<AgentChatEffort>,
    pub(crate) mode: Option<Mode>,
}

impl SelectionRequest {
    pub(crate) const fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.model.is_none()
            && self.effort.is_none()
            && self.mode.is_none()
    }

    pub(crate) fn needs_catalog(&self, base: Option<&AgentChatSelection>) -> bool {
        match base {
            None => self.provider.is_none() || self.model.is_none(),
            Some(base) => self.model.is_none() && self.changes_provider(base),
        }
    }

    pub(crate) fn wants_catalog(&self, base: Option<&AgentChatSelection>) -> bool {
        self.needs_catalog(base)
            || (self.effort.is_none()
                && base.is_none_or(|base| {
                    self.changes_provider(base)
                        || self
                            .model
                            .as_ref()
                            .is_some_and(|model| model != &base.model)
                }))
    }

    fn changes_provider(&self, base: &AgentChatSelection) -> bool {
        self.provider
            .is_some_and(|requested| provider(requested) != base.provider)
    }

    pub(crate) fn resolve(
        self,
        base: Option<&AgentChatSelection>,
        catalog: Option<&ModelCatalog>,
    ) -> Result<AgentChatSelection, String> {
        let chosen_provider = self
            .provider
            .map(provider)
            .or(base.map(|selection| selection.provider))
            .or(catalog.map(|catalog| catalog.default_selection.provider))
            .ok_or(CATALOG_UNAVAILABLE)?;
        let model = match (self.model, base) {
            (Some(model), _) => model,
            (None, Some(base)) if base.provider == chosen_provider => base.model.clone(),
            (None, _) => default_model(catalog, chosen_provider).ok_or_else(|| {
                catalog.map_or_else(
                    || CATALOG_UNAVAILABLE.to_owned(),
                    |_| {
                        format!(
                            "Gentd has not listed a default {} model; pass --model",
                            provider_name(chosen_provider)
                        )
                    },
                )
            })?,
        };
        let effort = self.effort.unwrap_or_else(|| {
            fitted_effort(
                catalog,
                chosen_provider,
                &model,
                base.map(|base| base.effort),
            )
        });
        Ok(AgentChatSelection {
            provider: chosen_provider,
            model,
            effort,
            mode: self
                .mode
                .map(mode)
                .or(base.map(|selection| selection.mode))
                .unwrap_or(AgentChatMode::Agent),
        })
    }
}

const CATALOG_UNAVAILABLE: &str =
    "this gentd does not list models yet; pass both --provider and --model";

pub(crate) fn default_model(
    catalog: Option<&ModelCatalog>,
    provider: AgentChatProvider,
) -> Option<String> {
    let catalog = catalog?;
    if catalog.default_selection.provider == provider {
        return Some(catalog.default_selection.model.clone());
    }
    let entry = catalog
        .providers
        .iter()
        .find(|entry| entry.provider == provider)?;
    entry
        .models
        .iter()
        .find(|model| model.is_default)
        .or_else(|| entry.models.first())
        .map(|model| model.id.clone())
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
