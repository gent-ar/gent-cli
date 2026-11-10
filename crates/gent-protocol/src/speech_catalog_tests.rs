use serde_json::{Value, json};

use super::{SpeechCatalog, SpeechCatalogError, SpeechCatalogFrame};

fn document() -> Value {
    json!({
        "schema": 1,
        "cloudProviders": [
            {
                "id": "together",
                "label": "Together AI",
                "transcriptionUrl": "https://api.together.xyz/v1/audio/transcriptions",
                "modelsUrl": "https://api.together.xyz/v1/models",
                "auth": "bearer",
                "apiKeyHint": "tok-...",
                "defaultModelId": "openai/whisper-large-v3",
                "models": [{"id": "openai/whisper-large-v3", "label": "Whisper large v3"}]
            },
            {
                "id": "resemble",
                "label": "Resemble AI",
                "transcriptionUrl": "https://app.resemble.ai/api/v2/speech-to-text",
                "modelsUrl": null,
                "auth": "bearer",
                "apiKeyHint": "rs-...",
                "defaultModelId": null,
                "models": []
            }
        ],
        "localEngines": [{
            "id": "whisper",
            "label": "Whisper",
            "models": [{
                "id": "tiny.en",
                "size": "tiny",
                "label": "Tiny",
                "filename": "ggml-tiny.en.bin",
                "sizeBytes": 77_704_715,
                "url": "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-tiny.en.bin",
                "sha256": "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
                "englishOnly": true
            }]
        }]
    })
}

fn parse(value: &Value) -> Result<SpeechCatalog, SpeechCatalogError> {
    SpeechCatalog::from_json(&value.to_string())
}

fn with(pointer: &str, replacement: Value) -> Result<SpeechCatalog, SpeechCatalogError> {
    let mut value = document();
    *value.pointer_mut(pointer).unwrap() = replacement;
    parse(&value)
}

#[test]
fn a_complete_catalogue_parses_with_every_source() {
    let catalog = parse(&document()).unwrap();
    assert_eq!(
        catalog.cloud_providers[0].default_model_id.as_deref(),
        Some("openai/whisper-large-v3")
    );
    assert_eq!(catalog.cloud_providers.len(), 2);
    assert_eq!(catalog.cloud_providers[1].default_model_id, None);
    assert!(catalog.local_engines[0].models[0].english_only);
}

#[test]
fn unknown_fields_are_rejected_at_every_level() {
    for (pointer, field) in [
        ("", "defaultSelection"),
        ("/cloudProviders/0", "apiKey"),
        ("/localEngines/0/models/0", "apiKey"),
    ] {
        let mut value = document();
        value
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(
                field.into(),
                json!({"provider": "together", "model": "secret"}),
            );
        assert_eq!(
            parse(&value),
            Err(SpeechCatalogError::Malformed),
            "{pointer}"
        );
    }
}

#[test]
fn repeated_identifiers_are_rejected() {
    let mut providers = document();
    let first = providers["cloudProviders"][0].clone();
    providers["cloudProviders"][1] = first;
    assert_eq!(
        parse(&providers),
        Err(SpeechCatalogError::Duplicate("together".into()))
    );
    assert_eq!(
        with("/localEngines/0/id", json!("together")),
        Err(SpeechCatalogError::Duplicate("together".into()))
    );
    let mut models = document();
    let model = models["cloudProviders"][0]["models"][0].clone();
    models["cloudProviders"][0]["models"]
        .as_array_mut()
        .unwrap()
        .push(model);
    assert_eq!(
        parse(&models),
        Err(SpeechCatalogError::Duplicate(
            "openai/whisper-large-v3".into()
        ))
    );
}

#[test]
fn every_url_must_be_https() {
    for pointer in [
        "/cloudProviders/0/transcriptionUrl",
        "/cloudProviders/0/modelsUrl",
    ] {
        assert_eq!(
            with(pointer, json!("http://api.together.xyz/v1/models")),
            Err(SpeechCatalogError::InvalidEntry("together".into())),
            "{pointer}"
        );
    }
    assert_eq!(
        with(
            "/localEngines/0/models/0/url",
            json!("http://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin")
        ),
        Err(SpeechCatalogError::InvalidEntry("tiny.en".into()))
    );
}

#[test]
fn provider_defaults_must_name_a_listed_model() {
    assert_eq!(
        with("/cloudProviders/1/defaultModelId", json!("best")),
        Err(SpeechCatalogError::InvalidEntry("resemble".into()))
    );
    assert_eq!(
        with("/cloudProviders/0/defaultModelId", json!("whisper-1")),
        Err(SpeechCatalogError::InvalidEntry("together".into()))
    );
    assert_eq!(
        with("/cloudProviders/0/defaultModelId", Value::Null),
        Err(SpeechCatalogError::InvalidEntry("together".into()))
    );
}

#[test]
fn local_models_must_be_hash_pinned_files_served_under_their_own_name() {
    for (pointer, value) in [
        ("/localEngines/0/models/0/sha256", json!("921E4CF8")),
        ("/localEngines/0/models/0/filename", json!("ggml-tiny.bin")),
        (
            "/localEngines/0/models/0/filename",
            json!("../ggml-tiny.en.bin"),
        ),
        ("/localEngines/0/models/0/sizeBytes", json!(0)),
    ] {
        assert_eq!(
            with(pointer, value),
            Err(SpeechCatalogError::InvalidEntry("tiny.en".into())),
            "{pointer}"
        );
    }
}

#[test]
fn an_unknown_schema_is_refused() {
    assert_eq!(
        with("/schema", json!(2)),
        Err(SpeechCatalogError::UnsupportedSchema(2))
    );
}

#[test]
fn the_revision_follows_content_not_formatting() {
    let catalog = parse(&document()).unwrap();
    let pretty = serde_json::to_string_pretty(&document()).unwrap();
    let revision = catalog.revision();
    assert_eq!(revision.len(), 64);
    assert_eq!(
        SpeechCatalog::from_json(&pretty).unwrap().revision(),
        revision
    );
    let relabelled = with("/cloudProviders/0/label", json!("Together")).unwrap();
    assert_ne!(relabelled.revision(), revision);
}

#[test]
fn the_catalog_frame_carries_the_revision_of_its_own_content() {
    let catalog = parse(&document()).unwrap();
    let frame = SpeechCatalogFrame::SpeechCatalog {
        request_id: "speech-1".into(),
        revision: catalog.revision(),
        catalog: catalog.clone(),
    };
    frame.validate().unwrap();
    let wire = serde_json::to_value(&frame).unwrap();
    assert_eq!(wire["type"], "speechCatalog");
    assert_eq!(wire["body"]["requestId"], "speech-1");
    assert_eq!(
        serde_json::from_value::<SpeechCatalogFrame>(wire).unwrap(),
        frame
    );
    let stale = SpeechCatalogFrame::SpeechCatalog {
        request_id: "speech-1".into(),
        revision: "0".repeat(64),
        catalog,
    };
    assert_eq!(stale.validate(), Err(SpeechCatalogError::RevisionMismatch));
    let request = serde_json::json!({"type": "readSpeechCatalog", "body": {"requestId": " x"}});
    let request: SpeechCatalogFrame = serde_json::from_value(request).unwrap();
    assert_eq!(
        request.validate(),
        Err(SpeechCatalogError::InvalidRequestId)
    );
}
