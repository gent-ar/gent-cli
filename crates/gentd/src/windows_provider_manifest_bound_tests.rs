use super::*;
use serde_json::Value;

const ACTUAL: &[u8] = include_bytes!("../tests/fixtures/windows-unified-provider-resources.json");

#[test]
fn actual_three_provider_finite_closures_exceed_old_cap_and_are_admitted() {
    assert_eq!(ACTUAL.len(), 310_805);
    assert!(ACTUAL.len() > 262_144);
    for (provider, count) in [("codex", 680), ("claude", 676), ("claurst", 671)] {
        let parsed = parse(ACTUAL, provider).unwrap();
        assert_eq!(parsed.provider, provider);
        assert_eq!(parsed.files.len(), count);
    }
}

#[test]
fn exact_512k_is_accepted_but_one_byte_more_fails_before_json_decode() {
    assert_eq!(MAX_BYTES, 512 * 1024);
    let mut bytes = ACTUAL.to_vec();
    bytes.resize(usize::try_from(MAX_BYTES).unwrap(), b' ');
    assert!(parse(&bytes, "codex").is_ok());
    bytes.push(b' ');
    assert_eq!(
        parse(&bytes, "codex").unwrap_err(),
        "managed resource manifest exceeds its bound"
    );
    let malformed_oversize = vec![b'['; usize::try_from(MAX_BYTES).unwrap() + 1];
    assert_eq!(
        parse(&malformed_oversize, "codex").unwrap_err(),
        "managed resource manifest exceeds its bound"
    );
}

#[test]
fn wider_byte_envelope_does_not_widen_provider_file_or_os_library_caps() {
    let original: Value = serde_json::from_slice(ACTUAL).unwrap();
    let mut value = original.clone();
    value["providers"]
        .as_array_mut()
        .unwrap()
        .push(original["providers"][0].clone());
    assert!(parse(&serde_json::to_vec(&value).unwrap(), "codex").is_err());
    let mut value = original.clone();
    let files = value["providers"][0]["files"].as_array_mut().unwrap();
    for index in files.len()..1025 {
        files.push(serde_json::json!({"root":"powerShellRuntime","path":format!("extra-{index}.dll"),"sha256":"a".repeat(64)}));
    }
    assert!(parse(&serde_json::to_vec(&value).unwrap(), "codex").is_err());
    let mut value = original;
    value["providers"][0]["osLibraries"] = serde_json::json!(
        (0..129)
            .map(|i| format!("library-{i}.dll"))
            .collect::<Vec<_>>()
    );
    assert!(parse(&serde_json::to_vec(&value).unwrap(), "codex").is_err());
}
