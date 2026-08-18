use std::collections::BTreeMap;

use ed25519_dalek::{Signer, SigningKey};

use super::{
    ClaudeAuthorityEvidenceError, ClaudeAuthorityEvidencePayload, ClaudeEvidenceScenario,
    ClaudeEvidenceTransport, ClaudeScenarioProof, SignedClaudeAuthorityEvidence,
};
use crate::compatibility::TrustedKeySet;

const SCENARIOS: [ClaudeEvidenceScenario; 15] = [
    ClaudeEvidenceScenario::FullTurn,
    ClaudeEvidenceScenario::ToolUse,
    ClaudeEvidenceScenario::ToolError,
    ClaudeEvidenceScenario::Thinking,
    ClaudeEvidenceScenario::PermissionPrompt,
    ClaudeEvidenceScenario::PermissionPersistent,
    ClaudeEvidenceScenario::PlanMode,
    ClaudeEvidenceScenario::Subagent,
    ClaudeEvidenceScenario::Compaction,
    ClaudeEvidenceScenario::McpTool,
    ClaudeEvidenceScenario::Resume,
    ClaudeEvidenceScenario::Interrupt,
    ClaudeEvidenceScenario::Steer,
    ClaudeEvidenceScenario::UsageCost,
    ClaudeEvidenceScenario::MalformedTolerance,
];

fn record(key: &SigningKey) -> SignedClaudeAuthorityEvidence {
    let scenarios = SCENARIOS
        .into_iter()
        .map(|scenario| {
            (
                scenario,
                ClaudeScenarioProof {
                    provider_version: "2.1.233".into(),
                    platform: "macos-arm64".into(),
                    transport: ClaudeEvidenceTransport::StreamJson,
                    fixture_sha256: "a".repeat(64),
                    attestation_sha256: "b".repeat(64),
                    capture_run_id: "capture-1".into(),
                    malformed_diagnostic_sha256: (scenario
                        == ClaudeEvidenceScenario::MalformedTolerance)
                        .then(|| "c".repeat(64)),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let payload = ClaudeAuthorityEvidencePayload {
        schema_version: 1,
        provider: "claude".into(),
        expires_at_unix_seconds: 100,
        compatibility_manifest_sha256: "d".repeat(64),
        transcript_inventory_sha256: "e".repeat(64),
        coverage_manifest_sha256: "f".repeat(64),
        scenarios,
    };
    SignedClaudeAuthorityEvidence {
        key_id: "evidence".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn keys(key: &SigningKey) -> TrustedKeySet {
    let mut keys = TrustedKeySet::default();
    keys.trust("evidence", key.verifying_key());
    keys
}

#[test]
fn accepts_a_signed_complete_claude_matrix() {
    let key = SigningKey::from_bytes(&[1; 32]);
    let verified = record(&key).verify(&keys(&key), 100).unwrap();
    assert_eq!(
        verified
            .scenario(ClaudeEvidenceScenario::MalformedTolerance)
            .transport,
        ClaudeEvidenceTransport::StreamJson
    );
}

#[test]
fn rejects_missing_or_malformed_required_claude_proofs() {
    let key = SigningKey::from_bytes(&[2; 32]);
    let mut missing = record(&key);
    missing
        .payload
        .scenarios
        .remove(&ClaudeEvidenceScenario::Compaction);
    missing.signature_hex = hex::encode(
        key.sign(&serde_json::to_vec(&missing.payload).unwrap())
            .to_bytes(),
    );
    assert!(matches!(
        missing.verify(&keys(&key), 100),
        Err(ClaudeAuthorityEvidenceError::InvalidScenarioSet)
    ));
    let mut malformed = record(&key);
    malformed
        .payload
        .scenarios
        .get_mut(&ClaudeEvidenceScenario::MalformedTolerance)
        .unwrap()
        .malformed_diagnostic_sha256 = None;
    malformed.signature_hex = hex::encode(
        key.sign(&serde_json::to_vec(&malformed.payload).unwrap())
            .to_bytes(),
    );
    assert!(matches!(
        malformed.verify(&keys(&key), 100),
        Err(ClaudeAuthorityEvidenceError::InvalidProof {
            scenario: ClaudeEvidenceScenario::MalformedTolerance,
            field: "malformed_diagnostic_sha256"
        })
    ));
}

#[test]
fn rejects_expired_provider_mismatch_and_tampering() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let mut expired = record(&key);
    expired.payload.expires_at_unix_seconds = 99;
    expired.signature_hex = hex::encode(
        key.sign(&serde_json::to_vec(&expired.payload).unwrap())
            .to_bytes(),
    );
    assert!(matches!(
        expired.verify(&keys(&key), 100),
        Err(ClaudeAuthorityEvidenceError::Expired)
    ));
    let mut provider = record(&key);
    provider.payload.provider = "codex".into();
    provider.signature_hex = hex::encode(
        key.sign(&serde_json::to_vec(&provider.payload).unwrap())
            .to_bytes(),
    );
    assert!(matches!(
        provider.verify(&keys(&key), 100),
        Err(ClaudeAuthorityEvidenceError::ProviderMismatch)
    ));
    let mut tampered = record(&key);
    tampered.signature_hex = "00".repeat(64);
    assert!(matches!(
        tampered.verify(&keys(&key), 100),
        Err(ClaudeAuthorityEvidenceError::SignatureMismatch)
    ));
}

#[test]
fn rejects_noncanonical_digests_before_signature_use() {
    let key = SigningKey::from_bytes(&[4; 32]);
    let mut invalid = record(&key);
    invalid.payload.transcript_inventory_sha256 = "A".repeat(64);
    invalid.signature_hex = hex::encode(
        key.sign(&serde_json::to_vec(&invalid.payload).unwrap())
            .to_bytes(),
    );
    assert!(matches!(
        invalid.verify(&keys(&key), 100),
        Err(ClaudeAuthorityEvidenceError::InvalidDigest {
            field: "transcript_inventory_sha256"
        })
    ));
}
