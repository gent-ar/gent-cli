use ed25519_dalek::{Signer, SigningKey};
use gent_ports::PackageInstallPolicy;

use super::{
    PackagePolicy, PackagePolicyEntry, PackagePolicyError, SignedPackagePolicy, TrustedKeySet,
};
use crate::provider_platform::{PLATFORMS, host_platform};

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
        package_name: host_platform()
            .and_then(|platform| platform.package_name(provider))
            .unwrap_or_else(|| provider.into()),
        version: match provider {
            "codex" => format!("0.147.0-{}", host_platform().unwrap().npm),
            _ => "0.147.0".into(),
        },
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
        format!("@openai/codex@0.147.0-{}", host_platform().unwrap().npm)
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

#[test]
fn claude_is_approved_only_as_this_platforms_native_binary_package() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let native = signed(&key, entry("claude"));
    let verified = native.verify(&keys(&key), 100, NODE).unwrap();
    assert_eq!(
        verified
            .approved_package("claude", 100)
            .unwrap()
            .package_name,
        host_platform().unwrap().package_name("claude").unwrap()
    );
    let mut wrapper = entry("claude");
    wrapper.package_name = "@anthropic-ai/claude-code".into();
    assert_eq!(
        signed(&key, wrapper).verify_envelope(&keys(&key), 1),
        Err(PackagePolicyError::InvalidShape)
    );
    let mut other = entry("claude");
    other.package_name = foreign_platform().package_name("claude").unwrap();
    let other = signed(&key, other).verify(&keys(&key), 100, NODE).unwrap();
    assert!(other.approved_package("claude", 100).is_err());
}

fn foreign_platform() -> crate::provider_platform::ProviderPlatform {
    *PLATFORMS
        .iter()
        .find(|platform| Some(**platform) != host_platform())
        .unwrap()
}

#[test]
fn codex_is_approved_only_as_this_platforms_native_tarball() {
    let key = SigningKey::from_bytes(&[8; 32]);
    let native = signed(&key, entry("codex"))
        .verify(&keys(&key), 100, NODE)
        .unwrap();
    assert!(native.approved_package("codex", 100).is_ok());
    let mut shim = entry("codex");
    shim.version = "0.147.0".into();
    assert_eq!(
        signed(&key, shim).verify_envelope(&keys(&key), 1),
        Err(PackagePolicyError::InvalidShape)
    );
    let mut other = entry("codex");
    other.version = format!("0.147.0-{}", foreign_platform().npm);
    let other = signed(&key, other).verify(&keys(&key), 100, NODE).unwrap();
    assert!(other.approved_package("codex", 100).is_err());
}
