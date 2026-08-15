use std::path::Path;

use serde_yaml::Value;

use super::{validate, validate_evidence_manifest};

#[test]
fn rejects_unknown_feature_state() {
    let fixture = r"
schema_version: 3
dimensions: { providers: [claude, codex, claurst, copilot], transports: [local_ipc, paired_mux], platforms: [macos, linux, windows] }
provider_implementation: { copilot: unsupported }
features: { example: { state: invented, evidence: required, legacy_owner: owner } }
";
    let value: Value = serde_yaml::from_str(fixture).unwrap();
    assert!(validate(&value, Path::new("."), false).is_err());
}

#[test]
fn baseline_is_valid_but_cannot_transfer_authority_without_real_evidence() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/coverage-manifest.yml");
    assert!(validate_evidence_manifest(&path, false).is_ok());
    assert!(
        validate_evidence_manifest(&path, true)
            .unwrap_err()
            .contains("evidence_records are absent")
    );
}

#[test]
fn recorded_evidence_requires_all_declared_kinds_and_implementation() {
    let fixture = r"
schema_version: 3
dimensions: { providers: [claude, codex, claurst, copilot], transports: [local_ipc, paired_mux], platforms: [macos, linux, windows] }
provider_implementation: { claude: rust_native_driver, codex: rust_native_driver, claurst: private_external_provider_bridge, copilot: unsupported }
required_evidence: [baseline, cutover]
features: { example: { state: supported, evidence: required, legacy_owner: owner } }
evidence_records:
  - id: incomplete
    feature: example
    state: supported
    provider: claude
    provider_version: 1
    platform: macos
    transport: local_ipc
    status: passed
    provider_implementation: rust_native_driver
    evidence_paths: { baseline: Cargo.toml }
    ci_artifact: signed:test
";
    let value: Value = serde_yaml::from_str(fixture).unwrap();
    assert!(
        validate(&value, Path::new("."), false)
            .unwrap_err()
            .contains("missing cutover evidence")
    );
}

#[test]
fn expired_temporarily_unavailable_exception_is_rejected() {
    let fixture = r"
schema_version: 3
dimensions: { providers: [claude, codex, claurst, copilot], transports: [local_ipc, paired_mux], platforms: [macos, linux, windows] }
provider_implementation: { claude: rust_native_driver, codex: rust_native_driver, claurst: private_external_provider_bridge, copilot: unsupported }
required_evidence: [baseline]
features: { example: { state: temporarily_unavailable, evidence: required, legacy_owner: owner, exception_expiry: 2999-01-01 } }
evidence_records:
  - id: expired
    feature: example
    state: temporarily_unavailable
    provider: claude
    provider_version: 1
    platform: macos
    transport: local_ipc
    status: recorded_absent
    provider_implementation: rust_native_driver
    evidence_paths: { baseline: Cargo.toml }
    ci_artifact: signed:test
    exception_expiry: 1970-01-01
";
    let value: Value = serde_yaml::from_str(fixture).unwrap();
    assert!(
        validate(&value, Path::new("."), false)
            .unwrap_err()
            .contains("has expired")
    );
}
