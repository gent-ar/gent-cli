use std::{fmt::Write, path::PathBuf, time::Duration};

use gent_protocol::{
    LocalModelInstallState,
    model_catalog::{
        CatalogModel, ModelCatalog, ModelListing, ProviderAvailability, ProviderModelCatalog,
    },
};
use gent_types::{AgentChatEffort, AgentChatProvider};

use super::render::bytes;

pub(crate) async fn list(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<ModelCatalog, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + FIRST_LISTING_WAIT;
    loop {
        let catalog = crate::model_catalog_cli::read(data_dir.clone(), no_autostart).await?;
        let loading = catalog
            .providers
            .iter()
            .any(|provider| provider.listing == ModelListing::Loading);
        if !loading || tokio::time::Instant::now() >= deadline {
            return Ok(catalog);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

const FIRST_LISTING_WAIT: Duration = Duration::from_secs(30);

pub(crate) fn render_catalog(catalog: &ModelCatalog) -> String {
    let mut output = format!(
        "Default for new conversations: {} {}\n",
        provider_name(catalog.default_selection.provider),
        catalog.default_selection.model
    );
    for provider in &catalog.providers {
        render_provider(&mut output, provider);
    }
    output.push_str("\n* provider default model\n");
    output
}

fn render_provider(output: &mut String, provider: &ProviderModelCatalog) {
    let _ = writeln!(
        output,
        "\n{} ({}) · {}",
        provider.label,
        provider_name(provider.provider),
        provider_state(provider)
    );
    let id_width = column(provider, |model| model.id.chars().count());
    let label_width = column(provider, |model| model.label.chars().count());
    for model in &provider.models {
        let mut line = format!(
            "  {} {:<id_width$}  {:<label_width$}  efforts: {}",
            if model.is_default { '*' } else { ' ' },
            model.id,
            model.label,
            efforts(model)
        );
        if let Some(local) = &model.local {
            let _ = write!(
                line,
                "  {} · {}",
                bytes(local.size_bytes),
                install_state(&local.install)
            );
        }
        let _ = writeln!(output, "{}", line.trim_end());
    }
}

fn column(provider: &ProviderModelCatalog, width: impl Fn(&CatalogModel) -> usize) -> usize {
    provider.models.iter().map(width).max().unwrap_or(0)
}

fn provider_state(provider: &ProviderModelCatalog) -> String {
    let availability = match provider.availability {
        ProviderAvailability::Checking => "checking",
        ProviderAvailability::Ready => "ready",
        ProviderAvailability::NotInstalled => "not installed; the first prompt asks to install it",
        ProviderAvailability::SignedOut => "signed out",
    };
    match &provider.listing {
        ModelListing::Ready => availability.into(),
        ModelListing::Loading => format!("{availability}, loading models"),
        ModelListing::Failed { message } => {
            format!("{availability}, models unavailable: {message}")
        }
    }
}

fn efforts(model: &CatalogModel) -> String {
    if model.efforts.is_empty() {
        return "any".into();
    }
    let listed = model
        .efforts
        .iter()
        .map(|effort| effort_name(*effort))
        .collect::<Vec<_>>()
        .join(", ");
    match model.default_effort {
        Some(default) => format!("{listed} (default {})", effort_name(default)),
        None => listed,
    }
}

fn install_state(state: &LocalModelInstallState) -> String {
    match state {
        LocalModelInstallState::NotInstalled => "not installed".into(),
        LocalModelInstallState::Downloading {
            downloaded_bytes,
            total_bytes,
        } if *total_bytes > 0 => format!(
            "downloading {}%",
            (u128::from(*downloaded_bytes) * 100 / u128::from(*total_bytes)).min(100)
        ),
        LocalModelInstallState::Downloading { .. } => "downloading".into(),
        LocalModelInstallState::Ready { .. } => "ready".into(),
    }
}

const fn provider_name(provider: AgentChatProvider) -> &'static str {
    match provider {
        AgentChatProvider::Claurst => "gent",
        AgentChatProvider::Claude => "claude",
        AgentChatProvider::Codex => "codex",
    }
}

const fn effort_name(effort: AgentChatEffort) -> &'static str {
    match effort {
        AgentChatEffort::Low => "low",
        AgentChatEffort::Medium => "medium",
        AgentChatEffort::High => "high",
        AgentChatEffort::XHigh => "xhigh",
        AgentChatEffort::Max => "max",
        AgentChatEffort::Ultra => "ultra",
    }
}

#[cfg(test)]
#[path = "local_models_cli_catalog_tests.rs"]
mod tests;
