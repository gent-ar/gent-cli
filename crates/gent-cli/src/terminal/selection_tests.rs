use gent_protocol::{
    LocalModelInstallState,
    model_catalog::{
        CatalogModel, LocalModelAvailability, ModelCatalog, ModelCatalogSelection, ModelListing,
        ProviderAvailability, ProviderModelCatalog,
    },
};
use gent_types::{AgentChatEffort, AgentChatProvider};

use crate::terminal::state::{UiCommand, UiEffect, UiRequest, UiState};

fn model(id: &str, is_default: bool, efforts: Vec<AgentChatEffort>) -> CatalogModel {
    CatalogModel {
        id: id.into(),
        label: id.into(),
        description: None,
        is_default,
        default_effort: efforts.first().copied(),
        efforts,
        local: None,
    }
}

fn provider(
    provider: AgentChatProvider,
    label: &str,
    models: Vec<CatalogModel>,
) -> ProviderModelCatalog {
    ProviderModelCatalog {
        provider,
        label: label.into(),
        availability: ProviderAvailability::Ready,
        listing: ModelListing::Ready,
        models,
    }
}

pub(crate) fn catalog() -> ModelCatalog {
    let mut small = model("qwen3-1-7b-q4-k-m", true, vec![AgentChatEffort::Medium]);
    small.local = Some(LocalModelAvailability {
        size_bytes: 1,
        install: LocalModelInstallState::NotInstalled,
    });
    ModelCatalog {
        default_selection: ModelCatalogSelection {
            provider: AgentChatProvider::Claurst,
            model: "qwen3-1-7b-q4-k-m".into(),
        },
        providers: vec![
            provider(
                AgentChatProvider::Claurst,
                "Gent",
                vec![small, model("qwen3-8b-q4-k-m", false, vec![])],
            ),
            provider(
                AgentChatProvider::Claude,
                "Claude",
                vec![model("default", true, vec![AgentChatEffort::Low])],
            ),
            provider(
                AgentChatProvider::Codex,
                "Codex",
                vec![
                    model("gpt-5.6-luna", false, vec![AgentChatEffort::Low]),
                    model(
                        "gpt-6-astra",
                        true,
                        vec![AgentChatEffort::Medium, AgentChatEffort::Ultra],
                    ),
                ],
            ),
        ],
    }
}

fn type_command(state: &mut UiState, command: &str) -> UiEffect {
    for character in command.chars() {
        state.apply(UiCommand::Insert(character));
    }
    state.apply(UiCommand::SubmitPrompt)
}

#[test]
fn the_terminal_starts_from_the_gentd_default_and_lists_its_providers() {
    let mut state = UiState::new(Vec::new())
        .with_chat_input(true)
        .with_command_catalog(Some(crate::terminal::commands::tests::catalog()))
        .with_model_catalog(Some(catalog()));
    assert_eq!(state.selection().provider, AgentChatProvider::Claurst);
    assert_eq!(state.selection().model, "qwen3-1-7b-q4-k-m");

    state.apply(UiCommand::CycleProvider);
    let (_, providers, _) = state.picker_view().unwrap();
    assert_eq!(providers, ["Gent", "Claude", "Codex"]);
}

#[test]
fn a_provider_choice_uses_that_providers_catalog_default_and_carries_into_the_next_new_chat() {
    let mut state = UiState::new(Vec::new())
        .with_chat_input(true)
        .with_command_catalog(Some(crate::terminal::commands::tests::catalog()))
        .with_model_catalog(Some(catalog()));
    choose(&mut state, UiCommand::CycleProvider, 2);
    assert_eq!(state.selection().model, "gpt-6-astra");
    assert!(matches!(
        state.apply(UiCommand::CreateConversation),
        UiEffect::Request(UiRequest::Create {
            selection: Some(selection),
            ..
        }) if selection.model == "gpt-6-astra"
    ));
    assert_eq!(
        type_command(&mut state, "/new"),
        UiEffect::Request(UiRequest::InvokeCommand {
            conversation_id: None,
            name: "new".into(),
            arguments: String::new(),
            session_id: None,
        })
    );
}

fn choose(state: &mut UiState, picker: UiCommand, steps: usize) {
    state.apply(picker);
    for _ in 0..steps {
        state.apply(UiCommand::SelectNext);
    }
    state.apply(UiCommand::SubmitPrompt);
}

#[test]
fn without_a_catalog_no_model_choices_are_invented() {
    let mut state = UiState::new(Vec::new())
        .with_chat_input(true)
        .with_command_catalog(Some(crate::terminal::commands::tests::catalog()));
    state.apply(UiCommand::CycleModel);
    assert!(state.picker_view().is_none());
}

#[test]
fn choosing_a_model_moves_the_effort_to_one_that_model_accepts() {
    let mut state = UiState::new(Vec::new())
        .with_chat_input(true)
        .with_model_catalog(Some(catalog()));
    choose(&mut state, UiCommand::CycleProvider, 2);
    choose(&mut state, UiCommand::CycleEffort, 1);
    assert_eq!(state.selection().effort, AgentChatEffort::Ultra);
    state.apply(UiCommand::CycleModel);
    state.apply(UiCommand::SelectPrevious);
    state.apply(UiCommand::SubmitPrompt);
    assert_eq!(state.selection().model, "gpt-5.6-luna");
    assert_eq!(state.selection().effort, AgentChatEffort::Low);
}
