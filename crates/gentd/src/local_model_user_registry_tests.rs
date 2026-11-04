use std::path::{Path, PathBuf};

use super::{UserModelEntry, UserModelRegistry, model_id};
use crate::{
    local_model_catalog::{LocalAgentProfile, LocalModelSource},
    local_model_provisioning::user_file::UserModelFile,
};

fn entry(path: &Path, label: &str) -> UserModelEntry {
    UserModelEntry::new(
        UserModelFile {
            path: path.to_path_buf(),
            label: label.into(),
            size_bytes: 10,
            modified_unix_nanos: 7,
            context_tokens: 8_192,
            runtime_memory_bytes: 20,
            agent_profile: LocalAgentProfile::Compact,
        },
        "a".repeat(64),
    )
}

fn absolute(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

#[test]
fn ids_are_user_scoped_slugs_with_a_path_hash() {
    let path = absolute("models/Qwen.gguf");
    let id = model_id("Qwen3 1.7B (Q4_K_M) ★", &path);
    assert!(id.starts_with("user-qwen3-1-7b-q4-k-m-"), "{id}");
    assert_eq!(id.len(), "user-qwen3-1-7b-q4-k-m-".len() + 8);
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    );
    assert_eq!(id, model_id("Qwen3 1.7B (Q4_K_M) ★", &path));
    assert_ne!(
        id,
        model_id("Qwen3 1.7B (Q4_K_M) ★", &absolute("other.gguf"))
    );
    assert!(model_id("★★", &path).starts_with("user-"));
    assert_eq!(model_id("★★", &path).len(), "user-".len() + 8);
    assert!(model_id(&"x".repeat(400), &path).len() <= 5 + 48 + 1 + 8);
}

#[test]
fn saves_loads_and_replaces_entries_for_the_same_file() {
    let directory = tempfile::tempdir().unwrap();
    let registry = UserModelRegistry::open(directory.path());
    assert_eq!(registry.entries().unwrap(), Vec::new());
    let first = absolute("first.gguf");
    registry.add(entry(&first, "First")).unwrap();
    registry.add(entry(&first, "Renamed")).unwrap();
    registry
        .add(entry(&absolute("second.gguf"), "Second"))
        .unwrap();
    let reopened = UserModelRegistry::open(directory.path()).entries().unwrap();
    assert_eq!(
        reopened
            .iter()
            .map(|entry| entry.label.as_str())
            .collect::<Vec<_>>(),
        ["Renamed", "Second"]
    );
    let record = reopened[0].record();
    assert_eq!(record.provider_model_id, record.id);
    assert_eq!(record.chat_template_file, None);
    assert_eq!(
        record.source,
        LocalModelSource::UserFile {
            path: first,
            modified_unix_nanos: 7,
        }
    );
    assert!(registry.remove(&reopened[0].id).unwrap());
    assert!(!registry.remove(&reopened[0].id).unwrap());
    assert_eq!(registry.entries().unwrap().len(), 1);
}

#[test]
fn a_malformed_registry_is_never_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("user-models.json");
    for malformed in [
        "{".to_owned(),
        r#"{"schema":2,"models":[]}"#.to_owned(),
        r#"{"schema":1,"models":[],"extra":true}"#.to_owned(),
        serde_json::json!({"schema":1,"models":[{"id":"qwen3-1-7b-q4-k-m","label":"x","path":absolute("a.gguf"),"sizeBytes":1,"modifiedUnixNanos":1,"sha256":"a".repeat(64),"contextTokens":8192,"runtimeMemoryBytes":2,"agentProfile":"full"}]}).to_string(),
    ] {
        std::fs::write(&path, &malformed).unwrap();
        let registry = UserModelRegistry::open(directory.path());
        assert!(registry.entries().is_err());
        assert!(
            registry
                .add(entry(&absolute("model.gguf"), "Model"))
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), malformed);
    }
}
