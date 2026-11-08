use gent_types::AgentChatEffort;

use super::*;

fn ids(models: &[CatalogModel]) -> Vec<&str> {
    models.iter().map(|model| model.id.as_str()).collect()
}

#[test]
fn the_bundled_catalog_is_well_formed() {
    let parsed = serde_json::from_str::<Bundled>(BUNDLED);
    assert!(parsed.is_ok());
    let bundled = parsed.unwrap_or_default();
    for curated in bundled.claude.iter().chain(&bundled.codex) {
        assert_eq!(
            model(curated).efforts.len(),
            curated.efforts.len(),
            "{}",
            curated.id
        );
    }
}

const VERIFIED_CLAUDE: [&str; 4] = ["opus[1m]", "claude-fable-5-1[1m]", "sonnet", "haiku"];
const VERIFIED_CODEX: [&str; 4] = [
    "gpt-6-astra",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
];

#[test]
fn the_static_fallback_holds_only_ids_a_real_install_listed() {
    for (provider, verified) in [
        (AgentChatProvider::Claude, VERIFIED_CLAUDE),
        (AgentChatProvider::Codex, VERIFIED_CODEX),
    ] {
        let models = fallback(provider);
        let mut expected = verified.to_vec();
        expected.push(PROVIDER_DEFAULT_MODEL);
        assert_eq!(ids(&models), expected);
        assert_eq!(models.iter().filter(|model| model.is_default).count(), 1);
    }
    let astra = &fallback(AgentChatProvider::Codex)[0];
    assert!(astra.efforts.contains(&AgentChatEffort::Ultra));
    assert_eq!(astra.default_effort, Some(AgentChatEffort::Low));
}

fn live(id: &str) -> Vec<CatalogModel> {
    let mut model = provider_default_model();
    model.id = id.into();
    discovered(vec![model])
}

#[test]
fn a_live_listing_replaces_the_fallback_and_becomes_the_last_known_listing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(LAST_KNOWN_FILE);
    let last_known = LastKnownModels::open(path.clone());
    let listed = settle(AgentChatProvider::Codex, live("gpt-live"), &last_known);
    assert_eq!(ids(&listed), ["gpt-live", PROVIDER_DEFAULT_MODEL]);
    let reopened = LastKnownModels::open(path);
    assert_eq!(
        settle(AgentChatProvider::Codex, Vec::new(), &reopened),
        listed
    );
    assert_eq!(
        settle(AgentChatProvider::Claude, Vec::new(), &reopened),
        fallback(AgentChatProvider::Claude)
    );
}

#[test]
fn a_newer_live_listing_overwrites_the_last_known_listing() {
    let directory = tempfile::tempdir().unwrap();
    let last_known = LastKnownModels::open(directory.path().join(LAST_KNOWN_FILE));
    settle(AgentChatProvider::Claude, live("old"), &last_known);
    settle(AgentChatProvider::Claude, live("new"), &last_known);
    assert_eq!(
        ids(&settle(AgentChatProvider::Claude, Vec::new(), &last_known)),
        ["new", PROVIDER_DEFAULT_MODEL]
    );
}

#[test]
fn a_corrupt_last_known_file_falls_back_to_the_static_list() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(LAST_KNOWN_FILE);
    std::fs::write(&path, "not json").unwrap();
    assert_eq!(
        settle(
            AgentChatProvider::Codex,
            Vec::new(),
            &LastKnownModels::open(path)
        ),
        fallback(AgentChatProvider::Codex)
    );
}

#[test]
fn claurst_has_no_curated_fallback() {
    assert!(fallback(AgentChatProvider::Claurst).is_empty());
}

#[test]
fn an_empty_live_listing_stays_empty_for_the_service_to_settle() {
    assert!(discovered(Vec::new()).is_empty());
}

#[test]
fn live_models_replace_the_curated_fallback() {
    let mut model = provider_default_model();
    model.id = "future-provider-model".into();
    model.label = "Provider supplied name".into();
    model.description = Some("Provider supplied details".into());
    model.efforts = vec![AgentChatEffort::High];
    model.default_effort = Some(AgentChatEffort::High);

    let result = discovered(vec![model.clone()]);

    assert_eq!(result.len(), 2);
    assert_eq!(result[0], model);
    assert_eq!(result[1].id, PROVIDER_DEFAULT_MODEL);
    assert!(!result[1].is_default);
}

#[test]
fn provider_default_metadata_is_preserved_without_duplicate_alias() {
    let mut model = provider_default_model();
    model.label = "Provider recommended".into();
    model.efforts = vec![AgentChatEffort::Medium];
    assert_eq!(discovered(vec![model.clone()]), vec![model]);
}
