use std::time::{Duration, Instant};

use gent_protocol::model_catalog::{ModelCatalogSelection, ModelListing, ProviderAvailability};
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider};

use super::{ModelCatalogService, ProviderListing, eventually, gated, model};
use crate::{
    provider_launch_budget::ProviderLaunchError,
    runtime_facade::model_catalog::defaults::ModelCatalogDefaults,
};

fn listed(service: &ModelCatalogService) {
    service.read(false);
    eventually(service, |catalog| {
        catalog
            .providers
            .iter()
            .all(|provider| provider.listing != ModelListing::Loading)
    });
}

#[test]
fn an_unlisted_provider_never_forwards_the_raw_effort_after_a_bounded_wait() {
    let directory = tempfile::tempdir().unwrap();
    let (codex, _codex_gate) = gated(AgentChatProvider::Codex);
    let service =
        ModelCatalogService::new(vec![codex], ModelCatalogDefaults::open(directory.path()));
    let started = Instant::now();
    let fitted = service
        .fit(&gent_types::AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.5".into(),
            effort: AgentChatEffort::Ultra,
            mode: AgentChatMode::Agent,
        })
        .unwrap();
    assert_eq!(fitted.effort, AgentChatEffort::Medium);
    assert_eq!(fitted.model, "gpt-5.5");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn unsupported_efforts_move_to_the_nearest_listed_effort_instead_of_being_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let (claude, claude_gate) = gated(AgentChatProvider::Claude);
    let (codex, codex_gate) = gated(AgentChatProvider::Codex);
    let (local, local_gate) = gated(AgentChatProvider::Claurst);
    let service = ModelCatalogService::new(
        vec![claude, codex, local],
        ModelCatalogDefaults::open(directory.path()),
    );
    let selection = |provider, model: &str, effort| gent_types::AgentChatSelection {
        provider,
        model: model.into(),
        effort,
        mode: AgentChatMode::Ask,
    };
    let mut sonnet = model("sonnet", Some(AgentChatEffort::Medium));
    sonnet.efforts = vec![AgentChatEffort::Medium, AgentChatEffort::High];
    let mut haiku = model("haiku", None);
    haiku.efforts = Vec::new();
    claude_gate
        .send(Ok(ProviderListing {
            availability: ProviderAvailability::Ready,
            models: vec![sonnet, haiku],
        }))
        .unwrap();
    codex_gate
        .send(Err(ProviderLaunchError::Failed("codex exited".into())))
        .unwrap();
    local_gate
        .send(Ok(ProviderListing {
            availability: ProviderAvailability::Ready,
            models: vec![
                model("qwen3-1-7b-q4-k-m", None),
                model("qwen3-8b-q4-k-m", None),
            ],
        }))
        .unwrap();
    listed(&service);

    let unknown = service
        .fit(&selection(
            AgentChatProvider::Claurst,
            "not-a-model",
            AgentChatEffort::Low,
        ))
        .unwrap_err();
    assert_eq!(unknown.code, "selectionModelUnavailable");
    assert!(
        unknown
            .message
            .ends_with("choose one of: qwen3-1-7b-q4-k-m, qwen3-8b-q4-k-m")
    );
    for (requested, fitted) in [
        (AgentChatEffort::Low, AgentChatEffort::Medium),
        (AgentChatEffort::Medium, AgentChatEffort::Medium),
        (AgentChatEffort::High, AgentChatEffort::High),
        (AgentChatEffort::XHigh, AgentChatEffort::High),
        (AgentChatEffort::Ultra, AgentChatEffort::High),
    ] {
        assert_eq!(
            service
                .fit(&selection(AgentChatProvider::Claude, "sonnet", requested))
                .unwrap()
                .effort,
            fitted
        );
    }
    assert_eq!(
        service
            .fit(&selection(
                AgentChatProvider::Claude,
                "haiku",
                AgentChatEffort::Ultra
            ))
            .unwrap()
            .effort,
        AgentChatEffort::Ultra
    );
    assert_eq!(
        service
            .fit(&selection(
                AgentChatProvider::Codex,
                "anything",
                AgentChatEffort::Ultra
            ))
            .unwrap()
            .effort,
        AgentChatEffort::Medium
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
    listed(&service);
    for (model, effort) in [
        ("default", AgentChatEffort::Medium),
        ("haiku", AgentChatEffort::Ultra),
    ] {
        assert!(
            service
                .fit(&gent_types::AgentChatSelection {
                    provider: AgentChatProvider::Claude,
                    model: model.into(),
                    effort,
                    mode: AgentChatMode::Ask,
                })
                .is_ok()
        );
    }
}

#[test]
fn claude_and_codex_accept_unlisted_models_while_gent_stays_strict() {
    let directory = tempfile::tempdir().unwrap();
    let (claude, claude_gate) = gated(AgentChatProvider::Claude);
    let (codex, codex_gate) = gated(AgentChatProvider::Codex);
    let (local, local_gate) = gated(AgentChatProvider::Claurst);
    let service = ModelCatalogService::new(
        vec![claude, codex, local],
        ModelCatalogDefaults::open(directory.path()),
    );
    for (gate, listed) in [
        (claude_gate, "sonnet"),
        (codex_gate, "gpt-5.6"),
        (local_gate, "qwen3-1-7b-q4-k-m"),
    ] {
        gate.send(Ok(ProviderListing {
            availability: ProviderAvailability::Ready,
            models: vec![model(listed, Some(AgentChatEffort::Medium))],
        }))
        .unwrap();
    }
    listed(&service);
    let selection = |provider, model: &str| gent_types::AgentChatSelection {
        provider,
        model: model.into(),
        effort: AgentChatEffort::Ultra,
        mode: AgentChatMode::Agent,
    };
    let default = |provider, model: &str| ModelCatalogSelection {
        provider,
        model: model.into(),
    };

    for (provider, unlisted) in [
        (AgentChatProvider::Claude, "claude-custom-preview"),
        (AgentChatProvider::Codex, "gpt-custom-preview"),
    ] {
        assert!(service.fit(&selection(provider, unlisted)).is_ok());
        let saved = service.set_default(default(provider, unlisted)).unwrap();
        assert_eq!(saved.default_selection, default(provider, unlisted));
        assert_eq!(
            service
                .set_default(default(provider, " "))
                .unwrap_err()
                .code,
            "selectionModelUnavailable"
        );
    }

    assert_eq!(
        service
            .fit(&selection(AgentChatProvider::Claurst, "custom-local"))
            .unwrap_err()
            .code,
        "selectionModelUnavailable"
    );
    assert_eq!(
        service
            .set_default(default(AgentChatProvider::Claurst, "custom-local"))
            .unwrap_err()
            .code,
        "selectionModelUnavailable"
    );
    assert_eq!(
        ModelCatalogDefaults::open(directory.path()).selection(),
        default(AgentChatProvider::Codex, "gpt-custom-preview")
    );
}
