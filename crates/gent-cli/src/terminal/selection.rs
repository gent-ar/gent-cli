use gent_protocol::model_catalog::ModelCatalog;
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};

pub(super) fn providers(catalog: Option<&ModelCatalog>) -> Vec<(AgentChatProvider, String)> {
    catalog
        .map(|catalog| {
            catalog
                .providers
                .iter()
                .map(|entry| (entry.provider, entry.label.clone()))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn models(catalog: Option<&ModelCatalog>, provider: AgentChatProvider) -> Vec<String> {
    catalog
        .and_then(|catalog| {
            catalog
                .providers
                .iter()
                .find(|entry| entry.provider == provider)
        })
        .map(|entry| entry.models.iter().map(|model| model.id.clone()).collect())
        .unwrap_or_default()
}

pub(super) use crate::chat_cli::default_model;

pub(super) fn efforts(
    catalog: Option<&ModelCatalog>,
    selection: &AgentChatSelection,
) -> Vec<AgentChatEffort> {
    catalog
        .and_then(|catalog| {
            crate::model_catalog_cli::catalog_model(catalog, selection.provider, &selection.model)
        })
        .map(|model| model.efforts.clone())
        .filter(|efforts| !efforts.is_empty())
        .unwrap_or_else(|| vec![selection.effort])
}

pub(super) fn default_selection(catalog: &ModelCatalog) -> AgentChatSelection {
    let selection = &catalog.default_selection;
    let effort = catalog
        .providers
        .iter()
        .find(|entry| entry.provider == selection.provider)
        .and_then(|entry| {
            entry
                .models
                .iter()
                .find(|model| model.id == selection.model)
        })
        .and_then(|model| model.default_effort)
        .unwrap_or(AgentChatEffort::Medium);
    AgentChatSelection {
        provider: selection.provider,
        model: selection.model.clone(),
        effort,
        mode: AgentChatMode::Agent,
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
pub(super) mod tests;
