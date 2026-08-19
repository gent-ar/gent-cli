//! Shared signed-release test scaffolding for the ordinary-authority-release chain.
//!
//! Extracted from `ordinary_authority_release_tests.rs` so any test needing a real signed,
//! verifiable release (not just the release-verification tests themselves) can build one
//! without duplicating the Ed25519 signing/manifest construction. A child module of
//! `ordinary_authority_release` so it can see that module's private release types.

use std::{collections::BTreeMap, fs, path::Path};

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use gent_adapters::{
    codex_authority_evidence::{
        CodexAuthorityEvidencePayload, CodexEvidenceScenario, CodexEvidenceTransport,
        CodexScenarioProof, SignedCodexAuthorityEvidence,
    },
    compatibility::{
        CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet,
    },
    compatibility_cache::CachedCompatibilityManifest,
    package_policy::{PackagePolicy, PackagePolicyEntry, SignedPackagePolicy},
};

use super::*;

pub(crate) fn release(root: &SigningKey, node_digest: &str) -> SignedOrdinaryAuthorityRelease {
    let compatibility_key = SigningKey::from_bytes(&[3; 32]);
    let policy_key = SigningKey::from_bytes(&[4; 32]);
    let evidence_key = SigningKey::from_bytes(&[5; 32]);
    let compatibility = compatibility(&compatibility_key);
    let evidence = evidence(&evidence_key, &digest(&compatibility));
    let policy = policy(&policy_key, node_digest);
    let payload = OrdinaryAuthorityReleasePayload {
        version: VERSION,
        expires_at_unix_seconds: 100,
        revoked: false,
        compatibility,
        compatibility_keys: vec![key("compat", &compatibility_key)],
        package_policy: policy,
        package_policy_keys: vec![key("policy", &policy_key)],
        providers: vec![ProviderAuthorityRelease::Codex {
            evidence,
            evidence_keys: vec![key("evidence", &evidence_key)],
        }],
    };
    SignedOrdinaryAuthorityRelease {
        key_id: "root".into(),
        signature_hex: hex::encode(root.sign(&canonical_payload(&payload).unwrap()).to_bytes()),
        payload,
    }
}

pub(crate) fn revoked_release(
    root: &SigningKey,
    node_digest: &str,
) -> SignedOrdinaryAuthorityRelease {
    let mut release = release(root, node_digest);
    release.payload.revoked = true;
    release.signature_hex = hex::encode(
        root.sign(&canonical_payload(&release.payload).unwrap())
            .to_bytes(),
    );
    release
}

fn compatibility(key: &SigningKey) -> SignedCompatibilityManifest {
    let payload = CompatibilityManifest {
        manifest_version: 1,
        expires_at_unix_seconds: 100,
        entries: vec![CompatibilityEntry {
            id: "codex-0.1.0".into(),
            provider: "codex".into(),
            version: "0.1.0".into(),
            digest_sha256: "a".repeat(64),
            revoked: false,
        }],
    };
    SignedCompatibilityManifest {
        key_id: "compat".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn policy(key: &SigningKey, node_digest: &str) -> SignedPackagePolicy {
    let payload = PackagePolicy {
        policy_version: 1,
        expires_at_unix_seconds: 100,
        entries: vec![PackagePolicyEntry {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "0.1.0".into(),
            integrity: format!("sha512-{}", STANDARD.encode([0_u8; 64])),
            node_runtime_digest_sha256: node_digest.into(),
            terms_version: "terms-1".into(),
            revoked: false,
        }],
    };
    SignedPackagePolicy {
        key_id: "policy".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn evidence(key: &SigningKey, compatibility_digest: &str) -> SignedCodexAuthorityEvidence {
    let scenarios = [
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
    ]
    .into_iter()
    .map(|scenario| {
        (
            scenario,
            CodexScenarioProof {
                provider_version: "0.1.0".into(),
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
        compatibility_manifest_sha256: compatibility_digest.into(),
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

fn key(id: &str, key: &SigningKey) -> ReleaseVerificationKey {
    ReleaseVerificationKey {
        key_id: id.into(),
        public_key_hex: hex::encode(key.verifying_key().as_bytes()),
    }
}

pub(crate) fn root_keys(key: &SigningKey) -> BTreeMap<String, VerifyingKey> {
    [("root".into(), key.verifying_key())].into()
}

pub(crate) fn digest(manifest: &SignedCompatibilityManifest) -> String {
    let mut keys = TrustedKeySet::default();
    let key = SigningKey::from_bytes(&[3; 32]);
    keys.trust("compat", key.verifying_key());
    let cached = CachedCompatibilityManifest::verify(manifest.clone(), &keys, 10).unwrap();
    CompatibilityAssessment::configured(keys, cached, 10)
        .manifest_sha256()
        .unwrap()
}

pub(crate) fn runtime(root: &Path) -> AppNodeRuntimeLock {
    let bin = root.join("node/bin");
    fs::create_dir_all(&bin).unwrap();
    let node = bin.join("node");
    fs::write(&node, "node").unwrap();
    fs::write(bin.join("npm"), "npm").unwrap();
    let cli = root.join("node/lib/node_modules/npm/bin");
    fs::create_dir_all(&cli).unwrap();
    fs::write(cli.join("npm-cli.js"), "npm cli").unwrap();
    AppNodeRuntimeLock::capture(Some(node.into_os_string()), root).unwrap()
}
