use gent_protocol::model_catalog::{
    CatalogModel, ModelCatalog, ModelCatalogSelection, ModelListing, ProviderAvailability,
    ProviderModelCatalog,
};
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};

use super::super::{Mode, Provider};
use super::SelectionRequest;

fn model(id: &str, is_default: bool, default_effort: Option<AgentChatEffort>) -> CatalogModel {
    CatalogModel {
        id: id.into(),
        label: id.into(),
        description: None,
        is_default,
        efforts: Vec::new(),
        default_effort,
        local: None,
    }
}

fn catalog() -> ModelCatalog {
    ModelCatalog {
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
                models: vec![model("qwen3-1-7b-q4-k-m", true, None)],
            },
            ProviderModelCatalog {
                provider: AgentChatProvider::Claude,
                label: "Claude".into(),
                availability: ProviderAvailability::Ready,
                listing: ModelListing::Ready,
                models: vec![
                    model("opus", false, Some(AgentChatEffort::High)),
                    model("sonnet", true, Some(AgentChatEffort::Low)),
                ],
            },
        ],
    }
}

fn parent() -> AgentChatSelection {
    AgentChatSelection {
        provider: AgentChatProvider::Claurst,
        model: "qwen3-1-7b-q4-k-m".into(),
        effort: AgentChatEffort::High,
        mode: AgentChatMode::Ask,
    }
}

#[test]
fn provider_alone_selects_that_providers_catalog_default_model() {
    let request = SelectionRequest {
        provider: Some(Provider::Claude),
        ..SelectionRequest::default()
    };
    assert!(request.needs_catalog(None));
    let selection = request.resolve(None, Some(&catalog())).unwrap();
    assert_eq!(selection.provider, AgentChatProvider::Claude);
    assert_eq!(selection.model, "sonnet");
    assert_eq!(selection.effort, AgentChatEffort::Low);
    assert_eq!(selection.mode, AgentChatMode::Agent);
}

#[test]
fn mode_alone_keeps_gentds_default_provider_and_model() {
    let selection = SelectionRequest {
        mode: Some(Mode::Ask),
        ..SelectionRequest::default()
    }
    .resolve(None, Some(&catalog()))
    .unwrap();
    assert_eq!(selection.provider, AgentChatProvider::Claurst);
    assert_eq!(selection.model, "qwen3-1-7b-q4-k-m");
    assert_eq!(selection.mode, AgentChatMode::Ask);
}

#[test]
fn a_switch_changing_only_effort_inherits_everything_else_without_the_catalog() {
    let request = SelectionRequest {
        effort: Some(gent_types::AgentChatEffort::Low),
        ..SelectionRequest::default()
    };
    assert!(!request.wants_catalog(Some(&parent())));
    let selection = request.resolve(Some(&parent()), None).unwrap();
    assert_eq!(
        selection,
        AgentChatSelection {
            effort: AgentChatEffort::Low,
            ..parent()
        }
    );
}

#[test]
fn a_switch_to_another_provider_uses_its_default_model_and_keeps_the_mode() {
    let request = SelectionRequest {
        provider: Some(Provider::Claude),
        ..SelectionRequest::default()
    };
    assert!(request.needs_catalog(Some(&parent())));
    let selection = request.resolve(Some(&parent()), Some(&catalog())).unwrap();
    assert_eq!(selection.provider, AgentChatProvider::Claude);
    assert_eq!(selection.model, "sonnet");
    assert_eq!(selection.mode, AgentChatMode::Ask);
}

#[test]
fn without_a_catalog_a_partial_new_selection_explains_what_to_pass() {
    let error = SelectionRequest {
        provider: Some(Provider::Codex),
        ..SelectionRequest::default()
    }
    .resolve(None, None)
    .unwrap_err();
    assert!(error.contains("--provider and --model"), "{error}");
}

#[test]
fn gent_is_the_user_facing_name_of_the_local_provider() {
    assert_eq!(super::provider(Provider::Gent), AgentChatProvider::Claurst);
    assert_eq!(super::provider_name(AgentChatProvider::Claurst), "Gent");
}

fn efforts_model(id: &str, efforts: Vec<AgentChatEffort>) -> CatalogModel {
    CatalogModel {
        default_effort: efforts.first().copied(),
        efforts,
        ..model(id, false, None)
    }
}

fn catalog_with_efforts() -> ModelCatalog {
    let mut catalog = catalog();
    catalog.providers[1].models = vec![
        efforts_model(
            "opus",
            vec![AgentChatEffort::Medium, AgentChatEffort::Ultra],
        ),
        efforts_model("haiku", vec![AgentChatEffort::Low]),
    ];
    catalog
}

#[test]
fn a_model_switch_keeps_the_effort_the_new_model_accepts_and_otherwise_takes_its_default() {
    let base = AgentChatSelection {
        provider: AgentChatProvider::Claude,
        model: "opus".into(),
        effort: AgentChatEffort::Ultra,
        mode: AgentChatMode::Ask,
    };
    let to = |model: &str| SelectionRequest {
        model: Some(model.into()),
        ..SelectionRequest::default()
    };
    assert!(!to("haiku").needs_catalog(Some(&base)));
    assert!(to("haiku").wants_catalog(Some(&base)));
    let haiku = to("haiku")
        .resolve(Some(&base), Some(&catalog_with_efforts()))
        .unwrap();
    assert_eq!(haiku.effort, AgentChatEffort::Low);
    let medium_base = AgentChatSelection {
        effort: AgentChatEffort::Medium,
        model: "haiku".into(),
        ..base
    };
    let opus = to("opus")
        .resolve(Some(&medium_base), Some(&catalog_with_efforts()))
        .unwrap();
    assert_eq!(opus.effort, AgentChatEffort::Medium);
}

#[test]
fn a_new_conversation_with_an_explicit_model_takes_that_models_default_effort() {
    let request = SelectionRequest {
        provider: Some(Provider::Claude),
        model: Some("haiku".into()),
        ..SelectionRequest::default()
    };
    assert!(!request.needs_catalog(None));
    assert!(request.wants_catalog(None));
    assert_eq!(
        request
            .resolve(None, Some(&catalog_with_efforts()))
            .unwrap()
            .effort,
        AgentChatEffort::Low
    );
}

#[test]
fn an_explicit_effort_is_left_for_gentd_to_validate_against_the_model() {
    let selection = SelectionRequest {
        provider: Some(Provider::Claude),
        model: Some("haiku".into()),
        effort: Some(AgentChatEffort::Ultra),
        ..SelectionRequest::default()
    }
    .resolve(None, Some(&catalog_with_efforts()))
    .unwrap();
    assert_eq!(selection.effort, AgentChatEffort::Ultra);
}

#[test]
fn effort_words_parse_without_a_fixed_choice_list() {
    assert_eq!(super::parse_effort("xhigh"), Ok(AgentChatEffort::XHigh));
    assert_eq!(super::parse_effort("xHigh"), Ok(AgentChatEffort::XHigh));
    let error = super::parse_effort("turbo").unwrap_err();
    assert!(
        error.contains("`turbo` is not a reasoning effort"),
        "{error}"
    );
}
