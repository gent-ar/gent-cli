use std::fs;

use serde_yaml::{Mapping, Value};
use tempfile::TempDir;

use super::driver_link::validate_record_link;
use crate::{PUBLIC_PROVIDERS, REQUIRED_SCENARIOS};

fn live_metadata(vendor: &str, scenario: &str, status: &str) -> String {
    format!(
        r#"{{"vendor":"{vendor}","scenario":"{scenario}","capturedAt":"2026-08-15T12:00:00Z","cliVersion":"1","adapterSpecVersion":1,"appVersion":"1","prompt":"safe","repo":"fixture@abc","notes":"redacted","executablePath":"/usr/local/bin/{vendor}","executableDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","platform":"macos-aarch64","transport":"stdio","captureRunId":"run-1","attestationDigest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","status":"{status}","captureOrigin":"live_cli"}}"#
    )
}

fn write_inventory(directory: &TempDir) -> std::path::PathBuf {
    let mut cells = Vec::new();
    for vendor in PUBLIC_PROVIDERS {
        for scenario in REQUIRED_SCENARIOS {
            let name = format!("{vendor}-{scenario}.jsonl");
            let recorded = vendor == "claude" && scenario == "full_turn";
            let status = if recorded {
                "recorded"
            } else {
                "recorded_absent"
            };
            let mut metadata = live_metadata(vendor, scenario, status);
            if !recorded {
                metadata.pop();
                metadata.push_str(
                    ",\"absenceReason\":\"not exposed\",\"observedCommand\":\"probe\",\"observedResult\":\"absent\"}",
                );
            }
            let frame = if recorded {
                "\n{\"in\":{\"type\":\"frame\"},\"expect\":null}"
            } else {
                ""
            };
            fs::write(
                directory.path().join(&name),
                format!("{{\"meta\":{metadata}}}{frame}"),
            )
            .unwrap();
            cells.push(format!(
                "  - {{ vendor: {vendor}, scenario: {scenario}, state: {status}, path: {name} }}"
            ));
        }
    }
    let path = directory.path().join("manifest.yml");
    fs::write(
        &path,
        format!(
            "schema_version: 1\nvendors: [claude, codex]\nscenarios: [{}]\ncells:\n{}",
            REQUIRED_SCENARIOS.join(", "),
            cells.join("\n")
        ),
    )
    .unwrap();
    path
}

fn record(transcript: &str) -> Mapping {
    serde_yaml::from_str::<Value>(&format!(
        "provider: claude\nprovider_version: '1'\nplatform: macos\ndriver_transport: stdio\ndriver_transcript: {transcript}"
    ))
    .unwrap()
    .as_mapping()
    .unwrap()
    .clone()
}

#[test]
fn authority_record_links_to_matching_live_recording() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = write_inventory(&directory);
    assert!(
        validate_record_link(
            &record("claude-full_turn.jsonl"),
            "public-run",
            directory.path(),
            Some(&manifest),
        )
        .is_ok()
    );
}

#[test]
fn authority_record_rejects_absent_or_mismatched_transcripts() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = write_inventory(&directory);
    let absent = validate_record_link(
        &record("codex-tool_use.jsonl"),
        "public-run",
        directory.path(),
        Some(&manifest),
    )
    .unwrap_err();
    assert!(absent.contains("must reference a recorded"));
    let mut changed = record("claude-full_turn.jsonl");
    changed.insert(
        Value::String("provider_version".into()),
        Value::String("2".into()),
    );
    assert!(
        validate_record_link(&changed, "public-run", directory.path(), Some(&manifest))
            .unwrap_err()
            .contains("cliVersion does not match")
    );
}
