use std::{collections::BTreeMap, fs};

use ed25519_dalek::{Signer, SigningKey};
use gent_adapters::{
    codex_authority_evidence::{
        CodexAuthorityEvidencePayload, CodexEvidenceScenario, CodexEvidenceTransport,
        CodexScenarioProof, SignedCodexAuthorityEvidence,
    },
    compatibility::{CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet},
    compatibility_cache::CachedCompatibilityManifest,
};

use super::{CodexAuthorityPreflightError, load};
use crate::CompatibilityAssessment;

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
) -> SignedCodexAuthorityEvidence {
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
    let payload = CodexAuthorityEvidencePayload {
        schema_version: 1,
        provider: "codex".into(),
        expires_at_unix_seconds: 100,
        compatibility_manifest_sha256: compatibility.manifest_sha256().unwrap(),
        transcript_inventory_sha256: "d".repeat(64),
        coverage_manifest_sha256: "e".repeat(64),
        scenarios,
    };
    SignedCodexAuthorityEvidence {
        key_id: "evidence".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn write(record: &SignedCodexAuthorityEvidence) -> tempfile::TempPath {
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::write(file.path(), serde_json::to_vec(record).unwrap()).unwrap();
    file.into_temp_path()
}

fn key_spec(key: &SigningKey) -> String {
    format!("evidence:{}", hex::encode(key.verifying_key().as_bytes()))
}

#[test]
fn reads_a_signed_codex_record_pinned_to_the_exact_compatibility_cache() {
    let compatibility = compatibility();
    let key = SigningKey::from_bytes(&[4; 32]);
    let path = write(&record(&compatibility, &key));
    let loaded = load(&path, &[key_spec(&key)], &compatibility, 10).unwrap();
    assert_eq!(
        loaded.evidence().compatibility_manifest_sha256(),
        compatibility.manifest_sha256().unwrap()
    );
}

#[test]
fn rejects_missing_or_mismatched_compatibility_evidence() {
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
        Err(CodexAuthorityPreflightError::CompatibilityUnavailable)
    ));
    let mut mismatch = record(&compatibility, &key);
    mismatch.payload.compatibility_manifest_sha256 = "f".repeat(64);
    mismatch.signature_hex = hex::encode(
        key.sign(&serde_json::to_vec(&mismatch.payload).unwrap())
            .to_bytes(),
    );
    assert!(matches!(
        load(&write(&mismatch), &[key_spec(&key)], &compatibility, 10),
        Err(CodexAuthorityPreflightError::CompatibilityMismatch)
    ));
}

#[test]
fn rejects_unavailable_bad_key_oversized_and_tampered_records() {
    let compatibility = compatibility();
    let key = SigningKey::from_bytes(&[6; 32]);
    let missing = tempfile::tempdir().unwrap().path().join("absent.json");
    assert!(matches!(
        load(&missing, &[key_spec(&key)], &compatibility, 10),
        Err(CodexAuthorityPreflightError::RecordUnavailable)
    ));
    let path = write(&record(&compatibility, &key));
    assert!(matches!(
        load(&path, &["not-a-key".into()], &compatibility, 10),
        Err(CodexAuthorityPreflightError::InvalidKey)
    ));
    let mut tampered = record(&compatibility, &key);
    tampered.signature_hex = "00".repeat(64);
    assert!(matches!(
        load(&write(&tampered), &[key_spec(&key)], &compatibility, 10),
        Err(CodexAuthorityPreflightError::Evidence(_))
    ));
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::write(file.path(), vec![b'x'; 65_537]).unwrap();
    assert!(matches!(
        load(file.path(), &[key_spec(&key)], &compatibility, 10),
        Err(CodexAuthorityPreflightError::RecordTooLarge)
    ));
}
