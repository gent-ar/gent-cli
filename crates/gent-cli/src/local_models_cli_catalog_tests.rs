use gent_protocol::{
    LocalModelInstallState,
    model_catalog::{
        CatalogModel, LocalModelAvailability, ModelCatalog, ModelCatalogSelection, ModelListing,
        ProviderAvailability, ProviderModelCatalog,
    },
};
use gent_types::{AgentChatEffort, AgentChatProvider};

use super::render_catalog;

fn model(id: &str, label: &str, is_default: bool) -> CatalogModel {
    CatalogModel {
        id: id.into(),
        label: label.into(),
        description: None,
        is_default,
        efforts: Vec::new(),
        default_effort: None,
        local: None,
    }
}

fn local(id: &str, label: &str, is_default: bool, install: LocalModelInstallState) -> CatalogModel {
    CatalogModel {
        efforts: vec![AgentChatEffort::Medium],
        default_effort: Some(AgentChatEffort::Medium),
        local: Some(LocalModelAvailability {
            size_bytes: 1_282_439_264,
            install,
        }),
        ..model(id, label, is_default)
    }
}

#[test]
fn every_provider_is_listed_in_gentd_order_with_its_state_efforts_and_local_install() {
    let mut sonnet = model("sonnet", "Sonnet", false);
    sonnet.efforts = vec![AgentChatEffort::Low, AgentChatEffort::XHigh];
    sonnet.default_effort = Some(AgentChatEffort::Low);
    let catalog = ModelCatalog {
        default_selection: ModelCatalogSelection {
            provider: AgentChatProvider::Claurst,
            model: "qwen3-1-7b-q4-k-m".into(),
        },
        providers: vec![
            ProviderModelCatalog {
                provider: AgentChatProvider::Claurst,
                label: "Gent".into(),
                availability: ProviderAvailability::Ready,
                listing: ModelListing::Ready,
                models: vec![
                    local(
                        "qwen3-1-7b-q4-k-m",
                        "Qwen3 1.7B",
                        true,
                        LocalModelInstallState::Downloading {
                            downloaded_bytes: 641_219_632,
                            total_bytes: 1_282_439_264,
                        },
                    ),
                    local(
                        "qwen3-8b",
                        "Qwen3 8B",
                        false,
                        LocalModelInstallState::Ready {
                            size_bytes: 1_282_439_264,
                        },
                    ),
                ],
            },
            ProviderModelCatalog {
                provider: AgentChatProvider::Claude,
                label: "Claude".into(),
                availability: ProviderAvailability::NotInstalled,
                listing: ModelListing::Ready,
                models: vec![model("default", "Default", true)],
            },
            ProviderModelCatalog {
                provider: AgentChatProvider::Codex,
                label: "Codex".into(),
                availability: ProviderAvailability::SignedOut,
                listing: ModelListing::Failed {
                    message: "not signed in".into(),
                },
                models: vec![sonnet],
            },
        ],
    };

    assert_eq!(
        render_catalog(&catalog),
        "Default for new conversations: gent qwen3-1-7b-q4-k-m\n\
         \n\
         Gent (gent) · ready\n\
         \x20 * qwen3-1-7b-q4-k-m  Qwen3 1.7B  efforts: medium (default medium)  1.2 GiB · downloading 50%\n\
         \x20   qwen3-8b           Qwen3 8B    efforts: medium (default medium)  1.2 GiB · ready\n\
         \n\
         Claude (claude) · not installed; the first prompt asks to install it\n\
         \x20 * default  Default  efforts: any\n\
         \n\
         Codex (codex) · signed out, models unavailable: not signed in\n\
         \x20   sonnet  Sonnet  efforts: low, xhigh (default low)\n\
         \n\
         * provider default model\n"
    );
}
