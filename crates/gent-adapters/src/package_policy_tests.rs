use ed25519_dalek::{Signer, SigningKey};
use gent_ports::PackageInstallPolicy;

use super::{
    PackagePolicy, PackagePolicyEntry, PackagePolicyError, SignedPackagePolicy, TrustedKeySet,
};

const NODE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn signed(key: &SigningKey, entry: PackagePolicyEntry) -> SignedPackagePolicy {
    let payload = PackagePolicy {
        policy_version: 1,
        expires_at_unix_seconds: 100,
        entries: vec![entry],
    };
    SignedPackagePolicy {
        key_id: "test-key".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn entry(provider: &str) -> PackagePolicyEntry {
    PackagePolicyEntry {
        provider: provider.into(),
        package_name: match provider {
            "claude" => "@anthropic-ai/claude-code",
            _ => "@openai/codex",
        }
        .into(),
        version: "0.147.0".into(),
        integrity: format!("sha512-{}==", "A".repeat(86)),
        node_runtime_digest_sha256: NODE.into(),
        terms_version: "2026-01".into(),
        revoked: false,
    }
}

fn keys(key: &SigningKey) -> TrustedKeySet {
    let mut keys = TrustedKeySet::default();
    keys.trust("test-key", key.verifying_key());
    keys
}

#[test]
fn verified_policy_selects_only_exact_nonrevoked_runtime_bound_package() {
    let key = SigningKey::from_bytes(&[4; 32]);
    let signed = signed(&key, entry("codex"));
    let verified = signed.verify(&keys(&key), 100, NODE).unwrap();
    assert_eq!(
        verified.approved_package("codex", 100).unwrap().selector(),
        "@openai/codex@0.147.0"
    );
    assert!(verified.approved_package("claude", 100).is_err());
    assert!(
        signed
            .verify(&keys(&key), 100, "b".repeat(64))
            .unwrap()
            .approved_package("codex", 100)
            .is_err()
    );
    assert!(verified.approved_package("codex", 101).is_err());
}

#[test]
fn invalid_policy_shapes_fail_before_signature_use() {
    let key = SigningKey::from_bytes(&[5; 32]);
    let mut policy = signed(&key, entry("codex"));
    policy.payload.entries[0].package_name = "codex".into();
    assert_eq!(
        policy.verify_envelope(&keys(&key), 1),
        Err(PackagePolicyError::InvalidShape)
    );
    let mut policy = signed(&key, entry("codex"));
    policy.payload.entries[0].version = "latest".into();
    assert_eq!(
        policy.verify_envelope(&keys(&key), 1),
        Err(PackagePolicyError::InvalidShape)
    );
    let mut policy = signed(&key, entry("claurst"));
    policy.payload.entries[0].package_name = "claurst".into();
    assert_eq!(
        policy.verify_envelope(&keys(&key), 1),
        Err(PackagePolicyError::InvalidShape)
    );
    let mut policy = signed(&key, entry("codex"));
    policy.payload.entries.push(entry("codex"));
    assert_eq!(
        policy.verify_envelope(&keys(&key), 1),
        Err(PackagePolicyError::InvalidShape)
    );
}

#[test]
fn strict_deserialization_rejects_unknown_data_and_bad_signatures() {
    let key = SigningKey::from_bytes(&[6; 32]);
    let policy = signed(&key, entry("claude"));
    let mut value = serde_json::to_value(&policy).unwrap();
    value["unknown"] = serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<SignedPackagePolicy>(value).is_err());
    let mut policy = policy;
    policy.signature_hex = "F".repeat(128);
    assert_eq!(
        policy.verify_envelope(&keys(&key), 1),
        Err(PackagePolicyError::InvalidShape)
    );
}
