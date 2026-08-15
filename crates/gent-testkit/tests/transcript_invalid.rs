use std::fs;

use gent_testkit::{TranscriptError, load_public_driver_fixture, load_public_driver_fixtures};
use tempfile::TempDir;

const HEADER: &str = r#"{"meta":{"vendor":"codex","scenario":"smoke","capturedAt":"2026-08-15","cliVersion":"0.1.0","adapterSpecVersion":"1","appVersion":"0.1.0","prompt":"ping","repo":"gent-cli@abc","notes":"redacted"}}"#;

fn write(content: &str) -> (TempDir, std::path::PathBuf) {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("fixture.jsonl");
    fs::write(&path, content).unwrap();
    (directory, path)
}

#[test]
fn reports_read_empty_and_invalid_json_inputs() {
    let directory = TempDir::new().unwrap();
    assert!(matches!(
        load_public_driver_fixture(directory.path().join("missing.jsonl")),
        Err(TranscriptError::Read { .. })
    ));
    let (_directory, path) = write("\n\n");
    assert!(matches!(
        load_public_driver_fixture(path),
        Err(TranscriptError::Empty { .. })
    ));
    let (_directory, path) = write("not json");
    assert!(matches!(
        load_public_driver_fixture(path),
        Err(TranscriptError::Json { line: 1, .. })
    ));
}

#[test]
fn validates_every_header_shape_and_provenance_rule() {
    for (header, expected) in [
        (r#"{"in":{}}"#, "first line must contain"),
        (r#"{"meta":{},"in":{}}"#, "meta header cannot"),
        (
            r#"{"meta":{"vendor":"copilot","scenario":"x","capturedAt":"x","cliVersion":"x","adapterSpecVersion":"x","appVersion":"x","prompt":"x","repo":"x","notes":"x"}}"#,
            "meta.vendor",
        ),
        (
            r#"{"meta":{"vendor":"claude","scenario":"","capturedAt":"x","cliVersion":"x","adapterSpecVersion":"x","appVersion":"x","prompt":"x","repo":"x","notes":"x"}}"#,
            "meta.scenario",
        ),
    ] {
        let (_directory, path) = write(header);
        let error = load_public_driver_fixture(path).unwrap_err();
        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn validates_every_frame_shape_rule() {
    for (frame, expected) in [
        ("[]", "frame must be a JSON object"),
        (r#"{"meta":{},"in":{},"expect":null}"#, "meta is allowed"),
        (r#"{"expect":null}"#, "frame is missing in"),
        (r#"{"in":{}}"#, "frame is missing expect"),
        (r#"{"in":{},"expect":1}"#, "expect must be"),
        (
            r#"{"in":{},"expect":null,"expectFields":[]}"#,
            "expectFields must be",
        ),
    ] {
        let (_directory, path) = write(&format!("{HEADER}\n{frame}"));
        let error = load_public_driver_fixture(path).unwrap_err();
        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn detects_secrets_in_nested_objects_arrays_and_text_markers() {
    for (frame, expected_path) in [
        (
            r#"{"in":{"api-key":"redacted"},"expect":null}"#,
            "$.in.api-key",
        ),
        (
            r#"{"in":["safe",{"value":"Bearer not-for-logs"}],"expect":null}"#,
            "$.in[1].value",
        ),
        (
            r#"{"in":{"value":"github_pat_not_for_logs"},"expect":null}"#,
            "$.in.value",
        ),
    ] {
        let (_directory, path) = write(&format!("{HEADER}\n{frame}"));
        let error = load_public_driver_fixture(path).unwrap_err();
        assert!(matches!(error, TranscriptError::PossibleSecret { .. }));
        assert!(error.to_string().contains(expected_path));
    }
}

#[test]
fn rejects_invalid_following_json_and_unreadable_fixture_directory() {
    let (_directory, path) = write(&format!("{HEADER}\ninvalid"));
    assert!(matches!(
        load_public_driver_fixture(path),
        Err(TranscriptError::Json { line: 2, .. })
    ));
    assert!(matches!(
        load_public_driver_fixtures("does-not-exist"),
        Err(TranscriptError::Read { .. })
    ));
}
