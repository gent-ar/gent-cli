use std::collections::BTreeMap;

use ed25519_dalek::{Signer, SigningKey};

use super::{
    CodexAuthorityEvidenceError, CodexAuthorityEvidencePayload, CodexEvidenceScenario,
    CodexEvidenceTransport, CodexScenarioProof, SignedCodexAuthorityEvidence, TrustedKeySet,
};

const SCENARIOS: [CodexEvidenceScenario; 15] = [
    CodexEvidenceScenario::FullTurn,
    CodexEvidenceScenario::ToolUse,
    CodexEvidenceScenario::ToolError,
    CodexEvidenceScenario::Thinking,
    CodexEvidenceScenario::PermissionPrompt,
    CodexEvidenceScenario::PermissionPersistent,
    CodexEvidenceScenario::PlanMode,
    CodexEvidenceScenario::Subagent,
    CodexEvidenceScenario::Compaction,
    CodexEvidenceScenario::McpTool,
    CodexEvidenceScenario::Resume,
    CodexEvidenceScenario::Interrupt,
    CodexEvidenceScenario::Steer,
    CodexEvidenceScenario::UsageCost,
    CodexEvidenceScenario::MalformedTolerance,
];

fn payload() -> CodexAuthorityEvidencePayload {
    let scenarios = SCENARIOS
        .into_iter()
        .map(|scenario| {
            (
                scenario,
                CodexScenarioProof {
                    provider_version: "0.147.0".into(),
                    platform: "macos-arm64".into(),
                    transport: CodexEvidenceTransport::JsonRpc,
                    fixture_sha256: "a".repeat(64),
                    attestation_sha256: "b".repeat(64),
                    capture_run_id: "capture-1".into(),
                    malformed_diagnostic_sha256: (scenario
                        == CodexEvidenceScenario::MalformedTolerance)
                        .then(|| "c".repeat(64)),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    CodexAuthorityEvidencePayload {
        schema_version: 1,
        provider: "codex".into(),
        expires_at_unix_seconds: 100,
        compatibility_manifest_sha256: "d".repeat(64),
        transcript_inventory_sha256: "e".repeat(64),
        coverage_manifest_sha256: "f".repeat(64),
        scenarios,
    }
}

fn signed(
    payload: CodexAuthorityEvidencePayload,
    key: &SigningKey,
) -> SignedCodexAuthorityEvidence {
    SignedCodexAuthorityEvidence {
        key_id: "test".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn keys(key: &SigningKey) -> TrustedKeySet {
    let mut keys = TrustedKeySet::default();
    keys.trust("test", key.verifying_key());
    keys
}

#[test]
fn signed_complete_codex_record_verifies_without_external_effects() {
    let key = SigningKey::from_bytes(&[1; 32]);
    let verified = signed(payload(), &key).verify(&keys(&key), 100).unwrap();
    assert_eq!(verified.compatibility_manifest_sha256(), "d".repeat(64));
    assert_eq!(
        verified
            .scenario(CodexEvidenceScenario::MalformedTolerance)
            .malformed_diagnostic_sha256,
        Some("c".repeat(64))
    );
}

#[test]
fn record_rejects_incomplete_or_non_malformed_proof_sets() {
    let key = SigningKey::from_bytes(&[2; 32]);
    let mut incomplete = payload();
    incomplete
        .scenarios
        .remove(&CodexEvidenceScenario::MalformedTolerance);
    assert!(matches!(
        signed(incomplete, &key).verify(&keys(&key), 1),
        Err(CodexAuthorityEvidenceError::InvalidScenarioSet)
    ));
    let mut absent_diagnostic = payload();
    absent_diagnostic
        .scenarios
        .get_mut(&CodexEvidenceScenario::MalformedTolerance)
        .unwrap()
        .malformed_diagnostic_sha256 = None;
    assert!(matches!(
        signed(absent_diagnostic, &key).verify(&keys(&key), 1),
        Err(CodexAuthorityEvidenceError::InvalidProof {
            scenario: CodexEvidenceScenario::MalformedTolerance,
            field: "malformed_diagnostic_sha256"
        })
    ));
}

#[test]
fn record_rejects_wrong_scope_expiry_and_noncanonical_digests() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let mut wrong_provider = payload();
    wrong_provider.provider = "claude".into();
    assert!(matches!(
        signed(wrong_provider, &key).verify(&keys(&key), 1),
        Err(CodexAuthorityEvidenceError::ProviderMismatch)
    ));
    assert!(matches!(
        signed(payload(), &key).verify(&keys(&key), 101),
        Err(CodexAuthorityEvidenceError::Expired)
    ));
    let mut bad_digest = payload();
    bad_digest.compatibility_manifest_sha256 = "A".repeat(64);
    assert!(matches!(
        signed(bad_digest, &key).verify(&keys(&key), 1),
        Err(CodexAuthorityEvidenceError::InvalidDigest {
            field: "compatibility_manifest_sha256"
        })
    ));
}

#[test]
fn record_rechecks_signature_and_trusted_key_status() {
    let key = SigningKey::from_bytes(&[4; 32]);
    let mut tampered = signed(payload(), &key);
    tampered.payload.coverage_manifest_sha256 = "a".repeat(64);
    assert!(matches!(
        tampered.verify(&keys(&key), 1),
        Err(CodexAuthorityEvidenceError::SignatureMismatch)
    ));
    let record = signed(payload(), &key);
    assert!(matches!(
        record.verify(&TrustedKeySet::default(), 1),
        Err(CodexAuthorityEvidenceError::Trust(_))
    ));
    let mut revoked = keys(&key);
    revoked.revoke("test");
    assert!(matches!(
        record.verify(&revoked, 1),
        Err(CodexAuthorityEvidenceError::Trust(_))
    ));
}

#[test]
fn record_rejects_oversized_or_noncanonical_signature_and_control_text() {
    let key = SigningKey::from_bytes(&[5; 32]);
    let mut signature = signed(payload(), &key);
    signature.signature_hex = "F".repeat(128);
    assert!(matches!(
        signature.verify(&keys(&key), 1),
        Err(CodexAuthorityEvidenceError::InvalidSignature)
    ));
    let mut control_text = payload();
    control_text
        .scenarios
        .get_mut(&CodexEvidenceScenario::FullTurn)
        .unwrap()
        .capture_run_id = "capture\n1".into();
    assert!(matches!(
        signed(control_text, &key).verify(&keys(&key), 1),
        Err(CodexAuthorityEvidenceError::InvalidProof {
            scenario: CodexEvidenceScenario::FullTurn,
            field: "capture_run_id"
        })
    ));
}
