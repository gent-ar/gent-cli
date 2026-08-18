use std::{collections::BTreeMap, fs};

use ed25519_dalek::{Signer, SigningKey};
use gent_adapters::{
    claude_authority_evidence::{
        ClaudeAuthorityEvidencePayload, ClaudeEvidenceScenario, ClaudeEvidenceTransport,
        ClaudeScenarioProof, SignedClaudeAuthorityEvidence,
    },
    compatibility::{CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet},
    compatibility_cache::CachedCompatibilityManifest,
};

use super::{ClaudeAuthorityPreflightError, load};
use crate::CompatibilityAssessment;

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

fn compatibility() -> CompatibilityAssessment {
    let key = SigningKey::from_bytes(&[8; 32]);
    let payload = CompatibilityManifest {
        manifest_version: 1,
        expires_at_unix_seconds: 100,
        entries: Vec::new(),
    };
    let manifest = SignedCompatibilityManifest {
        key_id: "compatibility".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    };
    let mut keys = TrustedKeySet::default();
    keys.trust("compatibility", key.verifying_key());
    CompatibilityAssessment::configured(
        keys.clone(),
        CachedCompatibilityManifest::verify(manifest, &keys, 1).unwrap(),
        10,
    )
}

fn record(
    compatibility: &CompatibilityAssessment,
    key: &SigningKey,
) -> SignedClaudeAuthorityEvidence {
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
        compatibility_manifest_sha256: compatibility.manifest_sha256().unwrap(),
        transcript_inventory_sha256: "d".repeat(64),
        coverage_manifest_sha256: "e".repeat(64),
        scenarios,
    };
    SignedClaudeAuthorityEvidence {
        key_id: "evidence".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn key_spec(key: &SigningKey) -> String {
    format!("evidence:{}", hex::encode(key.verifying_key().as_bytes()))
}
fn write(record: &SignedClaudeAuthorityEvidence) -> tempfile::TempPath {
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::write(file.path(), serde_json::to_vec(record).unwrap()).unwrap();
    file.into_temp_path()
}

#[test]
fn reads_signed_claude_evidence_pinned_to_the_exact_compatibility_cache() {
    let compatibility = compatibility();
    let key = SigningKey::from_bytes(&[4; 32]);
    let loaded = load(
        &write(&record(&compatibility, &key)),
        &[key_spec(&key)],
        &compatibility,
        10,
    )
    .unwrap();
    assert_eq!(
        loaded.evidence().compatibility_manifest_sha256(),
        compatibility.manifest_sha256().unwrap()
    );
}

#[test]
fn rejects_missing_mismatched_or_expired_claude_evidence() {
    let compatibility = compatibility();
    let key = SigningKey::from_bytes(&[5; 32]);
    let path = write(&record(&compatibility, &key));
    assert!(matches!(
        load(
            &path,
            &[key_spec(&key)],
            &CompatibilityAssessment::default(),
            10
        ),
        Err(ClaudeAuthorityPreflightError::CompatibilityUnavailable)
    ));
    let mut mismatch = record(&compatibility, &key);
    mismatch.payload.compatibility_manifest_sha256 = "f".repeat(64);
    mismatch.signature_hex = hex::encode(
        key.sign(&serde_json::to_vec(&mismatch.payload).unwrap())
            .to_bytes(),
    );
    assert!(matches!(
        load(&write(&mismatch), &[key_spec(&key)], &compatibility, 10),
        Err(ClaudeAuthorityPreflightError::CompatibilityMismatch)
    ));
    assert!(matches!(
        load(&path, &[key_spec(&key)], &compatibility, 101),
        Err(ClaudeAuthorityPreflightError::Evidence(_))
    ));
}

#[test]
fn rejects_unavailable_bad_key_oversized_and_tampered_records() {
    let compatibility = compatibility();
    let key = SigningKey::from_bytes(&[6; 32]);
    let missing = tempfile::tempdir().unwrap().path().join("absent.json");
    assert!(matches!(
        load(&missing, &[key_spec(&key)], &compatibility, 10),
        Err(ClaudeAuthorityPreflightError::RecordUnavailable)
    ));
    let path = write(&record(&compatibility, &key));
    assert!(matches!(
        load(&path, &["not-a-key".into()], &compatibility, 10),
        Err(ClaudeAuthorityPreflightError::InvalidKey)
    ));
    let mut tampered = record(&compatibility, &key);
    tampered.signature_hex = "00".repeat(64);
    assert!(matches!(
        load(&write(&tampered), &[key_spec(&key)], &compatibility, 10),
        Err(ClaudeAuthorityPreflightError::Evidence(_))
    ));
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::write(file.path(), vec![b'x'; 65_537]).unwrap();
    assert!(matches!(
        load(file.path(), &[key_spec(&key)], &compatibility, 10),
        Err(ClaudeAuthorityPreflightError::RecordTooLarge)
    ));
}
