use std::path::Path;

use serde_yaml::Value;

use super::validate;

const DIMENSIONS: &str = "dimensions: { providers: [claude, codex, claurst, copilot], transports: [local_ipc, paired_mux], platforms: [macos, linux, windows] }\nprovider_implementation: { claude: rust_native_driver, codex: rust_native_driver, claurst: private_external_provider_bridge, copilot: unsupported }";

fn manifest(feature: &str, evidence: &str) -> String {
    format!(
        "schema_version: 3\n{DIMENSIONS}\nrequired_evidence: [baseline]\nfeatures: {feature}\nevidence_records:\n{evidence}"
    )
}

fn valid_record() -> &'static str {
    "  - id: record\n    feature: example\n    state: supported\n    provider: claude\n    provider_version: 1\n    platform: macos\n    transport: local_ipc\n    status: passed\n    provider_implementation: rust_native_driver\n    evidence_paths: { baseline: Cargo.toml }\n    ci_artifact: signed:local"
}

fn validate_text(text: &str, authority_transfer: bool) -> Result<(), String> {
    let value: Value = serde_yaml::from_str(text).unwrap();
    validate(&value, Path::new("."), authority_transfer)
}

fn error_with(record: &str) -> String {
    let fixture = manifest(
        "{ example: { state: supported, evidence: required, legacy_owner: owner } }",
        record,
    );
    validate_text(&fixture, false).unwrap_err()
}

fn error_with_evidence_value(value: &str) -> String {
    let fixture = format!(
        "schema_version: 3\n{DIMENSIONS}\nrequired_evidence: [baseline]\nfeatures: {{ example: {{ state: supported, evidence: required, legacy_owner: owner }} }}\nevidence_records: {value}"
    );
    validate_text(&fixture, false).unwrap_err()
}

#[test]
fn validates_a_complete_non_authority_record() {
    let fixture = manifest(
        "{ example: { state: supported, evidence: required, legacy_owner: owner } }",
        valid_record(),
    );
    assert!(validate_text(&fixture, false).is_ok());
}

#[test]
fn rejects_duplicate_ids_unknown_features_and_state_mismatches() {
    let duplicate = format!("{}\n{}", valid_record(), valid_record());
    assert!(error_with(&duplicate).contains("duplicate evidence record id"));
    assert!(
        error_with(&valid_record().replacen("example", "missing", 1))
            .contains("missing required key missing")
    );
    assert!(
        error_with(&valid_record().replacen("state: supported", "state: emulated", 1))
            .contains("does not match")
    );
}

#[test]
fn rejects_unknown_dimension_members() {
    for (from, to, expected) in [
        (
            "provider: claude",
            "provider: unknown",
            "provider has unknown",
        ),
        (
            "platform: macos",
            "platform: solaris",
            "platform has unknown",
        ),
        (
            "transport: local_ipc",
            "transport: http",
            "transport has unknown",
        ),
    ] {
        assert!(error_with(&valid_record().replacen(from, to, 1)).contains(expected));
    }
}

#[test]
fn rejects_incomplete_or_invalid_records() {
    assert!(error_with_evidence_value("{}").contains("evidence_records must be a sequence"));
    assert!(error_with_evidence_value("[]").contains("evidence_records must not be empty"));
    assert!(
        error_with(&valid_record().replacen(
            "provider_version: 1",
            "provider_version: not_applicable",
            1
        ))
        .contains("has no provider version")
    );
    assert!(
        error_with(&valid_record().replacen("status: passed", "status: pending", 1))
            .contains("unknown status")
    );
    assert!(
        error_with(&valid_record().replacen(
            "provider_implementation: rust_native_driver",
            "provider_implementation: unsupported",
            1
        ))
        .contains("wrong provider implementation")
    );
    assert!(
        error_with(&valid_record().replacen("ci_artifact: signed:local", "ci_artifact: local", 1))
            .contains("ci_artifact must be a signed")
    );
}

#[test]
fn rejects_missing_or_unsafe_evidence_paths() {
    assert!(
        error_with(&valid_record().replacen("{ baseline: Cargo.toml }", "{}", 1))
            .contains("has no evidence paths")
    );
    assert!(
        error_with(&valid_record().replacen("Cargo.toml", "../Cargo.toml", 1))
            .contains("missing or non-repository path")
    );
    assert!(
        error_with(&valid_record().replacen("baseline: Cargo.toml", "'': Cargo.toml", 1))
            .contains("empty evidence kind")
    );

    let fixture = manifest(
        "{ example: { state: supported, evidence: required, legacy_owner: owner } }",
        &valid_record().replacen(
            "{ baseline: Cargo.toml }",
            "{ baseline: Cargo.toml, cutover: Cargo.toml }",
            1,
        ),
    )
    .replacen(
        "required_evidence: [baseline]",
        "required_evidence: [baseline, cutover]",
        1,
    );
    assert!(validate_text(&fixture, false).is_ok());

    let missing_kind = manifest(
        "{ example: { state: supported, evidence: required, legacy_owner: owner } }",
        valid_record(),
    )
    .replacen(
        "required_evidence: [baseline]",
        "required_evidence: [baseline, cutover]",
        1,
    );
    assert!(
        validate_text(&missing_kind, false)
            .unwrap_err()
            .contains("missing cutover evidence")
    );
}

#[test]
fn validates_private_and_exception_specific_requirements() {
    let claurst = valid_record()
        .replacen("provider: claude", "provider: claurst", 1)
        .replacen(
            "provider_implementation: rust_native_driver",
            "provider_implementation: private_external_provider_bridge",
            1,
        );
    assert!(error_with(&claurst).contains("missing required key private_evidence"));
    assert!(
        error_with(&format!("{claurst}\n    private_evidence: unsigned"))
            .contains("requires signed private Claurst evidence")
    );
    let fixture = manifest(
        "{ example: { state: supported, evidence: required, legacy_owner: owner } }",
        &format!("{claurst}\n    private_evidence: signed:bridge"),
    );
    assert!(validate_text(&fixture, false).is_ok());

    let temporary_feature = "{ example: { state: temporarily_unavailable, evidence: required, legacy_owner: owner, exception_expiry: 2999-01-01 } }";
    let temporary_record = valid_record()
        .replacen("state: supported", "state: temporarily_unavailable", 1)
        .replacen("status: passed", "status: recorded_absent", 1);
    let invalid = format!("{temporary_record}\n    exception_expiry: 2026-02-29");
    let fixture = manifest(temporary_feature, &invalid);
    assert!(
        validate_text(&fixture, false)
            .unwrap_err()
            .contains("YYYY-MM-DD")
    );
}

#[test]
fn authority_transfer_requires_terminal_records_and_legacy_fields() {
    let fixture = manifest(
        "{ example: { state: supported, evidence: required, legacy_owner: owner } }",
        &valid_record().replacen("status: passed", "status: failed", 1),
    );
    assert!(
        validate_text(&fixture, true)
            .unwrap_err()
            .contains("is failed")
    );

    let fixture = manifest(
        "{ example: { state: supported, evidence: required, legacy_owner: owner } }",
        valid_record(),
    );
    assert!(
        validate_text(&fixture, true)
            .unwrap_err()
            .contains("missing required key legacy_path")
    );

    let legacy = format!(
        "{}\n    legacy_path: app/path\n    removal_approval: signed:approval\n    removal_release: 1.0.0",
        valid_record()
    );
    let fixture = manifest(
        "{ example: { state: supported, evidence: required, legacy_owner: owner } }",
        &legacy,
    );
    assert!(
        validate_text(&fixture, true)
            .unwrap_err()
            .contains("missing evidence")
    );
}
