use std::fs;

use ed25519_dalek::{Signer, SigningKey};
use gent_ports::PackageInstallPolicy;

use super::{PackagePolicyArtifact, PackagePolicyArtifactError};
use crate::{
    compatibility::TrustedKeySet,
    package_policy::{PackagePolicy, PackagePolicyEntry, SignedPackagePolicy},
};

const NODE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn artifact_revalidates_and_binds_the_current_node_digest() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let keys = keys(&key);
    let artifact = PackagePolicyArtifact::from_verified(policy(&key), &keys, 10).unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("package-policy.json");
    fs::write(&path, serde_json::to_vec(&artifact).unwrap()).unwrap();
    let bound = PackagePolicyArtifact::load_bound(&path, &keys, 10, NODE).unwrap();
    assert!(bound.approved_package("codex", 10).is_ok());
    assert!(
        PackagePolicyArtifact::load_bound(&path, &keys, 10, "b".repeat(64))
            .unwrap()
            .approved_package("codex", 10)
            .is_err()
    );
    assert!(PackagePolicyArtifact::load_bound(&path, &keys, 101, NODE).is_err());
}

#[test]
fn artifact_rejects_nonregular_oversized_and_malformed_files() {
    let key = SigningKey::from_bytes(&[8; 32]);
    let keys = keys(&key);
    let root = tempfile::tempdir().unwrap();
    assert!(matches!(
        PackagePolicyArtifact::load_bound(root.path(), &keys, 1, NODE),
        Err(PackagePolicyArtifactError::NotRegular)
    ));
    let large = root.path().join("large.json");
    fs::write(&large, vec![b'x'; 65_537]).unwrap();
    assert!(matches!(
        PackagePolicyArtifact::load_bound(&large, &keys, 1, NODE),
        Err(PackagePolicyArtifactError::TooLarge)
    ));
    let malformed = root.path().join("malformed.json");
    fs::write(&malformed, "{").unwrap();
    assert!(matches!(
        PackagePolicyArtifact::load_bound(&malformed, &keys, 1, NODE),
        Err(PackagePolicyArtifactError::Malformed)
    ));
    let unknown = root.path().join("unknown.json");
    let mut value =
        serde_json::to_value(PackagePolicyArtifact::from_verified(policy(&key), &keys, 1).unwrap())
            .unwrap();
    value["unexpected"] = serde_json::Value::Bool(true);
    fs::write(&unknown, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        PackagePolicyArtifact::load_bound(&unknown, &keys, 1, NODE),
        Err(PackagePolicyArtifactError::Malformed)
    ));
}

#[cfg(unix)]
#[test]
fn artifact_rejects_a_symlink_even_when_its_target_is_valid() {
    use std::os::unix::fs::symlink;

    let key = SigningKey::from_bytes(&[9; 32]);
    let keys = keys(&key);
    let artifact = PackagePolicyArtifact::from_verified(policy(&key), &keys, 1).unwrap();
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target.json");
    fs::write(&target, serde_json::to_vec(&artifact).unwrap()).unwrap();
    let link = root.path().join("link.json");
    symlink(&target, &link).unwrap();
    assert!(matches!(
        PackagePolicyArtifact::load_bound(&link, &keys, 1, NODE),
        Err(PackagePolicyArtifactError::NotRegular)
    ));
}

fn keys(key: &SigningKey) -> TrustedKeySet {
    let mut keys = TrustedKeySet::default();
    keys.trust("test-key", key.verifying_key());
    keys
}

fn policy(key: &SigningKey) -> SignedPackagePolicy {
    let payload = PackagePolicy {
        policy_version: 1,
        expires_at_unix_seconds: 100,
        entries: vec![PackagePolicyEntry {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "0.147.0".into(),
            integrity: format!("sha512-{}==", "A".repeat(86)),
            node_runtime_digest_sha256: NODE.into(),
            terms_version: "2026-01".into(),
            revoked: false,
        }],
    };
    SignedPackagePolicy {
        key_id: "test-key".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}
