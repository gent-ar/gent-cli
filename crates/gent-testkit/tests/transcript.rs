use gent_testkit::{TranscriptError, load_public_driver_fixture, load_public_driver_fixtures};
use std::fs;
use tempfile::TempDir;

const HEADER: &str = r#"{"meta":{"vendor":"claude","scenario":"full_turn","capturedAt":"2026-08-15","cliVersion":"2.0.0","adapterSpecVersion":"28","appVersion":"0.1.0","prompt":"Say hello","repo":"fixture@abc","notes":"fixture repository"}}"#;

#[test]
fn loads_a_realistically_shaped_redacted_recording() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("turn.jsonl");
    fs::write(
        &path,
        format!("{HEADER}\n{{\"in\":{{\"type\":\"text\"}},\"expect\":\"AgentStreamTextDelta\",\"expectFields\":{{\"text\":\"hello\"}}}}\n"),
    )
    .unwrap();

    let fixture = load_public_driver_fixture(&path).unwrap();
    assert_eq!(fixture.metadata["vendor"], "claude");
    assert_eq!(fixture.frames.len(), 1);
    assert_eq!(
        fixture.frames[0].expectation.as_deref(),
        Some("AgentStreamTextDelta")
    );
}

#[test]
fn accepts_the_documented_numeric_adapter_spec_version() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("numeric-version.jsonl");
    let content = HEADER.replacen(
        "\"adapterSpecVersion\":\"28\"",
        "\"adapterSpecVersion\":28",
        1,
    );
    fs::write(&path, content).unwrap();
    assert!(load_public_driver_fixture(path).is_ok());
}

#[test]
fn rejects_missing_provenance_and_unredacted_tokens_without_echoing_them() {
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("missing.jsonl");
    fs::write(&missing, r#"{"in":{},"expect":null}"#).unwrap();
    assert!(matches!(
        load_public_driver_fixture(missing),
        Err(TranscriptError::Invalid { line: 1, .. })
    ));

    let token = directory.path().join("token.jsonl");
    fs::write(
        &token,
        format!("{HEADER}\n{{\"in\":{{\"value\":\"sk-live-not-for-logs\"}},\"expect\":null}}"),
    )
    .unwrap();
    let error = load_public_driver_fixture(token).unwrap_err();
    assert!(matches!(
        error,
        TranscriptError::PossibleSecret { line: 2, .. }
    ));
    assert!(!error.to_string().contains("live-not-for-logs"));
}

#[test]
fn rejects_sensitive_field_names_and_orders_directory_fixtures() {
    let directory = TempDir::new().unwrap();
    let first = directory.path().join("a.jsonl");
    let second = directory.path().join("b.jsonl");
    fs::write(
        &second,
        format!("{HEADER}\n{{\"in\":{{}},\"expect\":null}}"),
    )
    .unwrap();
    fs::write(&first, format!("{HEADER}\n{{\"in\":{{}},\"expect\":null}}")).unwrap();
    fs::write(directory.path().join("ignored.txt"), "not a recording").unwrap();

    let fixtures = load_public_driver_fixtures(directory.path()).unwrap();
    assert_eq!(fixtures.len(), 2);
    assert!(fixtures[0].path.ends_with("a.jsonl"));

    let sensitive = directory.path().join("sensitive.jsonl");
    fs::write(
        &sensitive,
        format!("{HEADER}\n{{\"in\":{{\"authorization\":\"redacted\"}},\"expect\":null}}"),
    )
    .unwrap();
    assert!(matches!(
        load_public_driver_fixture(sensitive),
        Err(TranscriptError::PossibleSecret { line: 2, .. })
    ));
}
