use std::fs;

use gent_testkit::{PUBLIC_PROVIDERS, REQUIRED_SCENARIOS, validate_public_driver_manifest};
use tempfile::TempDir;

fn manifest(cells: &str) -> String {
    format!(
        "schema_version: 1\nvendors: [claude, codex]\nscenarios: [{}]\ncells:\n{cells}",
        REQUIRED_SCENARIOS.join(", ")
    )
}

fn all_cells(state: &str) -> String {
    PUBLIC_PROVIDERS
        .into_iter()
        .flat_map(|vendor| {
            REQUIRED_SCENARIOS.map(move |scenario| {
                format!("  - {{ vendor: {vendor}, scenario: {scenario}, state: {state} }}")
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_manifest(directory: &TempDir, cells: &str) -> std::path::PathBuf {
    let path = directory.path().join("manifest.yml");
    fs::write(&path, manifest(cells)).unwrap();
    path
}

fn write_fixture(directory: &TempDir, name: &str, metadata: &str, frame: bool) {
    let body = if frame {
        format!("{{\"meta\":{metadata}}}\n{{\"in\":{{\"type\":\"frame\"}},\"expect\":null}}")
    } else {
        format!("{{\"meta\":{metadata}}}")
    };
    fs::write(directory.path().join(name), body).unwrap();
}

fn live_metadata(provider: &str, scenario: &str, extra: &str) -> String {
    format!(
        r#"{{"vendor":"{provider}","scenario":"{scenario}","capturedAt":"2026-08-15T12:00:00Z","cliVersion":"1","adapterSpecVersion":1,"appVersion":"1","prompt":"safe","repo":"fixture@abc","notes":"redacted","executablePath":"/usr/local/bin/{provider}","executableDigest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","platform":"macos-aarch64","transport":"stdio","captureRunId":"run-1","attestationDigest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"{extra}}}"#,
    )
}

fn recorded_absent_cells(directory: &TempDir) -> String {
    let mut cells = Vec::new();
    for provider in PUBLIC_PROVIDERS {
        for scenario in REQUIRED_SCENARIOS {
            let name = format!("{provider}-{scenario}.jsonl");
            let extra = format!(
                r#","status":"recorded_absent","captureOrigin":"live_cli","absenceReason":"probe did not expose scenario","observedCommand":"{provider} --version","observedResult":"scenario unavailable""#
            );
            write_fixture(
                directory,
                &name,
                &live_metadata(provider, scenario, &extra),
                false,
            );
            cells.push(format!(
                "  - {{ vendor: {provider}, scenario: {scenario}, state: recorded_absent, path: {name} }}"
            ));
        }
    }
    cells.join("\n")
}

#[test]
fn capture_inventory_is_valid_but_live_mode_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    let path = write_manifest(&directory, &all_cells("capture_required"));
    assert!(validate_public_driver_manifest(&path, false).is_ok());
    assert!(
        validate_public_driver_manifest(&path, true)
            .unwrap_err()
            .contains("live transcript capture is still required")
    );
}

#[test]
fn rejects_missing_duplicate_and_unknown_cells() {
    let directory = tempfile::tempdir().unwrap();
    let mut cells = all_cells("capture_required");
    cells.push_str("\n  - { vendor: claude, scenario: full_turn, state: capture_required }");
    cells.push_str("\n  - { vendor: copilot, scenario: full_turn, state: capture_required }");
    let path = write_manifest(&directory, &cells);
    let error = validate_public_driver_manifest(&path, false).unwrap_err();
    assert!(error.contains("duplicate transcript matrix cell claude/full_turn"));
    assert!(error.contains("unknown transcript matrix cell copilot/full_turn"));
}

#[test]
fn validates_recorded_and_reasoned_absent_fixture_shapes() {
    let directory = tempfile::tempdir().unwrap();
    write_fixture(
        &directory,
        "claude-full.jsonl",
        &live_metadata(
            "claude",
            "full_turn",
            r#","status":"recorded","captureOrigin":"live_cli""#,
        ),
        true,
    );
    write_fixture(
        &directory,
        "codex-absent.jsonl",
        &live_metadata(
            "codex",
            "tool_use",
            r#","status":"recorded_absent","captureOrigin":"live_cli","absenceReason":"unsupported by installed version","observedCommand":"codex --help","observedResult":"flag unavailable""#,
        ),
        false,
    );
    write_fixture(
        &directory,
        "claude-synthetic.jsonl",
        &live_metadata(
            "claude",
            "tool_error",
            r#","status":"synthetic","captureOrigin":"synthetic""#,
        ),
        true,
    );
    let mut cells = all_cells("capture_required");
    cells = cells.replacen(
        "{ vendor: claude, scenario: full_turn, state: capture_required }",
        "{ vendor: claude, scenario: full_turn, state: recorded, path: claude-full.jsonl }",
        1,
    );
    cells = cells.replacen(
        "{ vendor: codex, scenario: tool_use, state: capture_required }",
        "{ vendor: codex, scenario: tool_use, state: recorded_absent, path: codex-absent.jsonl }",
        1,
    );
    cells = cells.replacen(
        "{ vendor: claude, scenario: tool_error, state: capture_required }",
        "{ vendor: claude, scenario: tool_error, state: synthetic, path: claude-synthetic.jsonl, rationale: deterministic parser fixture }",
        1,
    );
    let path = write_manifest(&directory, &cells);
    assert!(validate_public_driver_manifest(&path, false).is_ok());
    let error = validate_public_driver_manifest(&path, true).unwrap_err();
    assert!(error.contains("synthetic transcript cannot satisfy live evidence"));
    assert!(error.contains("live transcript capture is still required"));
}

#[test]
fn rejects_duplicate_dimensions_and_paths_outside_the_manifest_root() {
    let directory = tempfile::tempdir().unwrap();
    let cells = all_cells("capture_required").replacen(
        "{ vendor: claude, scenario: full_turn, state: capture_required }",
        "{ vendor: claude, scenario: full_turn, state: synthetic, path: ../outside.jsonl, rationale: test }",
        1,
    );
    let path = write_manifest(&directory, &cells);
    let duplicate = fs::read_to_string(&path).unwrap().replacen(
        "vendors: [claude, codex]",
        "vendors: [claude, codex, codex]",
        1,
    );
    fs::write(&path, duplicate).unwrap();
    let error = validate_public_driver_manifest(&path, false).unwrap_err();
    assert!(error.contains("vendors must declare exactly"));
    assert!(error.contains("fixture path must be repository-relative"));
}

#[test]
fn strict_mode_accepts_complete_reasoned_live_probe_matrix() {
    let directory = tempfile::tempdir().unwrap();
    let path = write_manifest(&directory, &recorded_absent_cells(&directory));
    assert!(validate_public_driver_manifest(&path, true).is_ok());
}

#[test]
fn rejects_missing_files_and_invalid_live_provenance() {
    let directory = tempfile::tempdir().unwrap();
    let bad = live_metadata(
        "claude",
        "tool_error",
        r#","status":"recorded","captureOrigin":"live_cli""#,
    )
    .replacen("sha256:aaaa", "not-a-digest", 1);
    write_fixture(&directory, "bad.jsonl", &bad, true);
    let mut cells = all_cells("capture_required");
    cells = cells.replacen(
        "{ vendor: claude, scenario: full_turn, state: capture_required }",
        "{ vendor: claude, scenario: full_turn, state: recorded, path: missing.jsonl }",
        1,
    );
    cells = cells.replacen(
        "{ vendor: claude, scenario: tool_use, state: capture_required }",
        "{ vendor: claude, scenario: tool_use, state: recorded }",
        1,
    );
    cells = cells.replacen(
        "{ vendor: claude, scenario: tool_error, state: capture_required }",
        "{ vendor: claude, scenario: tool_error, state: recorded, path: bad.jsonl }",
        1,
    );
    cells = cells.replacen(
        "{ vendor: claude, scenario: thinking, state: capture_required }",
        "{ vendor: claude, scenario: thinking, state: recorded, path: invalid.txt }",
        1,
    );
    let error =
        validate_public_driver_manifest(&write_manifest(&directory, &cells), false).unwrap_err();
    assert!(error.contains("could not resolve fixture"));
    assert!(error.contains("fixture path is required"));
    assert!(error.contains("fixture path must be repository-relative"));
    assert!(error.contains("invalid or missing executableDigest"));
}
