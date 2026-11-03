use serde_json::json;

use super::claude_model;

#[test]
fn the_provider_default_alias_and_unknown_future_models_preserve_their_real_metadata() {
    let entries = [
        json!({"value": "default", "displayName": "Default (recommended)", "supportedEffortLevels": ["low", "high"]}),
        json!({"value": "provider-future", "displayName": "Future family", "description": "Provider description", "isDefault": false,
            "supportedEffortLevels": ["low", "high"], "defaultEffortLevel": "high"}),
    ];
    let models = entries.iter().filter_map(claude_model).collect::<Vec<_>>();
    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        ["default", "provider-future"]
    );
    assert!(models[0].is_default);
    assert_eq!(models[0].label, "Default (recommended)");
    assert_eq!(models[0].default_effort, None);
    assert_eq!(models[1].label, "Future family");
    assert_eq!(
        models[1].description.as_deref(),
        Some("Provider description")
    );
    assert!(!models[1].is_default);
    assert_eq!(
        models[1].default_effort,
        Some(gent_types::AgentChatEffort::High)
    );
}

#[test]
fn missing_or_unsupported_effort_defaults_are_not_invented() {
    for entry in [
        json!({"value": "provider-one", "supportedEffortLevels": ["medium"]}),
        json!({"value": "provider-two", "supportedEffortLevels": ["low"], "defaultEffortLevel": "high"}),
    ] {
        assert_eq!(claude_model(&entry).unwrap().default_effort, None);
    }
}

#[test]
fn captured_managed_and_new_binary_catalogs_keep_every_reported_selector() {
    for capture in [
        include_str!("../fixtures/model_catalog/public-claude-2.1.271.json"),
        include_str!("../fixtures/model_catalog/public-claude-2.1.312.json"),
    ] {
        let fixture: serde_json::Value = serde_json::from_str(capture).unwrap();
        let models = fixture["pages"][0]["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(claude_model)
            .collect::<Vec<_>>();
        let ids = models
            .into_iter()
            .map(|model| serde_json::Value::String(model.id))
            .collect::<Vec<_>>();
        assert_eq!(ids, *fixture["expectedIds"].as_array().unwrap());
    }
}
