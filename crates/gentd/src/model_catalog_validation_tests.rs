use gent_protocol::model_catalog::ProviderAvailability;
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider};

use super::{ModelCatalogService, ProviderListing, gated, model};
use crate::{
    provider_launch_budget::ProviderLaunchError,
    runtime_facade::model_catalog::defaults::ModelCatalogDefaults,
};

#[test]
fn selections_are_checked_against_a_loaded_listing_with_the_valid_choices_named() {
    let directory = tempfile::tempdir().unwrap();
    let (claude, claude_gate) = gated(AgentChatProvider::Claude);
    let (codex, codex_gate) = gated(AgentChatProvider::Codex);
    let service = ModelCatalogService::new(
        vec![claude, codex],
        ModelCatalogDefaults::open(directory.path()),
    );
    let selection = |provider, model: &str, effort| gent_types::AgentChatSelection {
        provider,
        model: model.into(),
        effort,
        mode: AgentChatMode::Ask,
    };
    let mut sonnet = model("sonnet", Some(AgentChatEffort::Medium));
    sonnet.efforts = vec![AgentChatEffort::Low, AgentChatEffort::Medium];
    claude_gate
        .send(Ok(ProviderListing {
            availability: ProviderAvailability::Ready,
            models: vec![sonnet, model("haiku", None)],
        }))
        .unwrap();
    codex_gate
        .send(Err(ProviderLaunchError::Failed("codex exited".into())))
        .unwrap();

    let unknown = service
        .validate(&selection(
            AgentChatProvider::Claude,
            "not-a-model",
            AgentChatEffort::Low,
        ))
        .unwrap_err();
    assert_eq!(unknown.code, "selectionModelUnavailable");
    assert!(unknown.message.ends_with("choose one of: sonnet, haiku"));
    let effort = service
        .validate(&selection(
            AgentChatProvider::Claude,
            "sonnet",
            AgentChatEffort::Ultra,
        ))
        .unwrap_err();
    assert_eq!(effort.code, "selectionEffortUnavailable");
    assert!(effort.message.ends_with("choose one of: low, medium"));
    assert!(
        service
            .validate(&selection(
                AgentChatProvider::Claude,
                "sonnet",
                AgentChatEffort::Medium
            ))
            .is_ok()
    );
    assert!(
        service
            .validate(&selection(
                AgentChatProvider::Claude,
                "haiku",
                AgentChatEffort::High
            ))
            .is_ok()
    );
    assert!(
        service
            .validate(&selection(
                AgentChatProvider::Codex,
                "anything",
                AgentChatEffort::Ultra
            ))
            .is_ok()
    );
}

#[test]
fn a_provider_that_is_not_installed_accepts_any_model_so_the_prompt_can_be_held_for_install() {
    let directory = tempfile::tempdir().unwrap();
    let (claude, claude_gate) = gated(AgentChatProvider::Claude);
    let service =
        ModelCatalogService::new(vec![claude], ModelCatalogDefaults::open(directory.path()));
    claude_gate
        .send(Ok(ProviderListing {
            availability: ProviderAvailability::NotInstalled,
            models: vec![super::super::provider_default_model()],
        }))
        .unwrap();
    for (model, effort) in [
        ("default", AgentChatEffort::Medium),
        ("haiku", AgentChatEffort::Ultra),
    ] {
        assert!(
            service
                .validate(&gent_types::AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: model.into(),
                    effort,
                    mode: AgentChatMode::Ask,
                })
                .is_ok()
        );
    }
}
