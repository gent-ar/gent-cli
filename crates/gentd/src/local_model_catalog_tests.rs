use super::{LocalAgentProfile, LocalModelCatalog, LocalModelCatalogError};

#[test]
fn shipped_catalogue_is_strict_and_queryable() {
    let catalogue = LocalModelCatalog::shipped().unwrap();
    let qwen3 = Some("qwen3-tool-use.jinja");
    assert_eq!(
        catalogue
            .models()
            .iter()
            .map(|model| (
                model.id.as_str(),
                model.size_bytes,
                model.runtime_memory_bytes,
                model.agent_profile,
                model.chat_template_file.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            (
                "qwen3-1-7b-q4-k-m",
                1_282_439_264,
                4_294_967_296,
                LocalAgentProfile::Compact,
                qwen3
            ),
            (
                "llama-3-2-3b-instruct-q4-k-m",
                2_019_377_696,
                5_562_426_160,
                LocalAgentProfile::Full,
                None
            ),
            (
                "qwen3-4b-q4-k-m",
                2_497_280_256,
                6_849_705_344,
                LocalAgentProfile::Compact,
                qwen3
            ),
            (
                "qwen2-5-coder-7b-instruct-q4-k-m",
                4_683_073_536,
                8_559_725_568,
                LocalAgentProfile::Full,
                None
            ),
            (
                "hermes-3-llama-3-1-8b-q4-k-m",
                4_920_733_824,
                7_312_472_096,
                LocalAgentProfile::Full,
                None
            ),
            (
                "qwen3-8b-q4-k-m",
                5_027_783_488,
                7_704_734_432,
                LocalAgentProfile::Full,
                qwen3
            ),
            (
                "qwen2-5-coder-14b-instruct-q4-k-m",
                8_988_111_072,
                17_441_589_584,
                LocalAgentProfile::Full,
                None
            ),
            (
                "qwen3-14b-q4-k-m",
                9_001_752_960,
                16_891_627_072,
                LocalAgentProfile::Full,
                qwen3
            ),
            (
                "devstral-small-2507-q4-k-m",
                14_333_918_432,
                24_889_875_280,
                LocalAgentProfile::Full,
                None
            ),
        ]
    );
    assert!(
        catalogue
            .models()
            .iter()
            .all(|model| model.context_tokens == 32_768)
    );
    assert!(
        catalogue
            .model(gent_protocol::DEFAULT_LOCAL_MODEL_ID)
            .is_some()
    );
}

#[test]
fn rejects_unknown_fields_and_unsafe_downloads() {
    let unknown = r#"{"models":[{"id":"model","label":"Model","huggingface_url":"https://huggingface.co/a/b/resolve/0123456789abcdef0123456789abcdef01234567/a.gguf","local_filename":"a.gguf","provider_model_id":"model","size_bytes":1,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","context_tokens":8192,"runtime_memory_bytes":4096,"agent_profile":"full","extra":true}]}"#;
    assert_eq!(
        LocalModelCatalog::from_json(unknown),
        Err(LocalModelCatalogError::Malformed)
    );
    let unsafe_url = unknown
        .replace(",\"extra\":true", "")
        .replace("https://huggingface.co", "http://example.test");
    assert_eq!(
        LocalModelCatalog::from_json(&unsafe_url),
        Err(LocalModelCatalogError::InvalidRecord("model".into()))
    );
}

#[test]
fn rejects_duplicate_ids_and_unsafe_filenames() {
    let duplicate = r#"{"models":[{"id":"model","label":"Model","huggingface_url":"https://huggingface.co/a/b/resolve/0123456789abcdef0123456789abcdef01234567/a.gguf","local_filename":"a.gguf","provider_model_id":"model","size_bytes":1,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","context_tokens":8192,"runtime_memory_bytes":4096,"agent_profile":"full"},{"id":"model","label":"Model Two","huggingface_url":"https://huggingface.co/a/b/resolve/0123456789abcdef0123456789abcdef01234567/b.gguf","local_filename":"b.gguf","provider_model_id":"model-two","size_bytes":1,"sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","context_tokens":8192,"runtime_memory_bytes":4096,"agent_profile":"full"}]}"#;
    assert_eq!(
        LocalModelCatalog::from_json(duplicate),
        Err(LocalModelCatalogError::Duplicate("model".into()))
    );
    let unsafe_filename = duplicate
        .replace(
            r#""id":"model","label":"Model Two""#,
            r#""id":"model-two","label":"Model Two""#,
        )
        .replace("b.gguf", "../b.gguf");
    assert_eq!(
        LocalModelCatalog::from_json(&unsafe_filename),
        Err(LocalModelCatalogError::InvalidRecord("model-two".into()))
    );
}
