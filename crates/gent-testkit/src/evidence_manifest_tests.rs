use std::path::Path;

use serde_yaml::Value;

use super::validate;

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
    let value: Value =
        serde_yaml::from_str(include_str!("../../../fixtures/coverage-manifest.yml")).unwrap();
    assert!(validate(&value, Path::new("."), false).is_ok());
    assert!(
        validate(&value, Path::new("."), true)
            .unwrap_err()
            .contains("evidence_records are absent")
    );
}
