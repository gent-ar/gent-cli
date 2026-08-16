use std::fs;

use serde_yaml::{Mapping, Value};
use tempfile::TempDir;

use super::driver_link::{manifest_path, validate_record_link};
use crate::{PUBLIC_PROVIDERS, REQUIRED_SCENARIOS};

fn live_metadata(vendor: &str, scenario: &str, status: &str) -> String {
    format!(
        r#"{{"vendor":"{vendor}","scenario":"{scenario}","capturedAt":"2026-08-15T12:00:00Z","cliVersion":"1","adapterSpecVersion":1,"appVersion":"1","prompt":"safe","repo":"fixture@abc","notes":"redacted","executablePath":"/usr/local/bin/{vendor}","executableDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","platform":"macos-aarch64","transport":"stdio","captureRunId":"run-1","attestationDigest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","status":"{status}","captureOrigin":"live_cli"}}"#
    )
}

fn write_inventory(directory: &TempDir, include_absence: bool) -> std::path::PathBuf {
    let mut cells = Vec::new();
    for vendor in PUBLIC_PROVIDERS {
        for scenario in REQUIRED_SCENARIOS {
            let name = format!("{vendor}-{scenario}.jsonl");
            let recorded = !(include_absence && vendor == "codex" && scenario == "tool_use");
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
    let manifest = write_inventory(&directory, false);
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
    let manifest = write_inventory(&directory, true);
    let absent = validate_record_link(
        &record("codex-tool_use.jsonl"),
        "public-run",
        directory.path(),
        Some(&manifest),
    )
    .unwrap_err();
    assert!(absent.contains("recorded absence cannot satisfy live provider evidence"));
    let mut changed = record("claude-full_turn.jsonl");
    changed.insert(
        Value::String("provider_version".into()),
        Value::String("2".into()),
    );
    let clean_manifest = write_inventory(&directory, false);
    assert!(
        validate_record_link(
            &changed,
            "public-run",
            directory.path(),
            Some(&clean_manifest)
        )
        .unwrap_err()
        .contains("cliVersion does not match")
    );
}

#[test]
fn links_reject_missing_manifests_and_unsafe_transcript_paths() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = write_inventory(&directory, false);
    assert!(
        validate_record_link(
            &record("claude-full_turn.jsonl"),
            "run",
            directory.path(),
            None
        )
        .unwrap_err()
        .contains("has no public-driver manifest")
    );
    for path in ["../outside.jsonl", "/tmp/outside.jsonl", "missing.jsonl"] {
        assert!(
            validate_record_link(&record(path), "run", directory.path(), Some(&manifest)).is_err()
        );
    }
}

#[test]
fn links_reject_metadata_mismatches_and_manifest_escape_attempts() {
    let directory = tempfile::tempdir().unwrap();
    let manifest = write_inventory(&directory, false);
    for (field, value, expected) in [
        ("provider", "codex", "vendor does not match"),
        ("platform", "linux", "platform does not match"),
        ("driver_transport", "json_rpc", "transport does not match"),
    ] {
        let mut changed = record("claude-full_turn.jsonl");
        changed.insert(Value::String(field.into()), Value::String(value.into()));
        assert!(
            validate_record_link(&changed, "run", directory.path(), Some(&manifest))
                .unwrap_err()
                .contains(expected)
        );
    }
    let empty = Mapping::new();
    assert_eq!(manifest_path(&empty, directory.path()).unwrap(), None);
    for path in ["../manifest.yml", "/tmp/manifest.yml", "missing.yml"] {
        let pointer =
            serde_yaml::from_str::<Value>(&format!("public_driver_transcript_manifest: {path}"))
                .unwrap();
        assert!(manifest_path(pointer.as_mapping().unwrap(), directory.path()).is_err());
    }
}
