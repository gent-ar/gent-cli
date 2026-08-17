use std::collections::BTreeMap;

use ed25519_dalek::{Signer, SigningKey};
use gent_types::{
    RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact, RuntimeReleaseChannel,
    RuntimeReleaseManifest, RuntimeVersion, SignedRuntimeRelease,
};
use tempfile::tempdir;

use super::CachedRuntimeRelease;
use crate::RuntimeReleaseTrust;

fn release(key: &SigningKey, expiry: u64) -> SignedRuntimeRelease {
    let payload = RuntimeReleaseManifest {
        manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
        release_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        protocol_min: 1,
        protocol_max: 2,
        schema_min: 1,
        schema_max: 2,
        minimum_app_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        channel: RuntimeReleaseChannel::Stable,
        rollout_percent: 100,
        expires_at_unix_seconds: expiry,
        revoked: false,
        forward_only_schema: false,
        artifact: RuntimeReleaseArtifact {
            target: "aarch64-apple-darwin".into(),
            archive_name: "gent.tar.gz".into(),
            digest_sha256: "b".repeat(64),
            size_bytes: 1,
        },
    };
    SignedRuntimeRelease {
        key_id: "release-1".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn trust(key: &SigningKey) -> RuntimeReleaseTrust {
    RuntimeReleaseTrust::new(BTreeMap::from([("release-1".into(), key.verifying_key())]))
}

#[test]
fn stores_and_loads_a_revalidated_release() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let trust = trust(&key);
    let cache = CachedRuntimeRelease::verify(release(&key, 10), &trust, 1).unwrap();
    let directory = tempdir().unwrap();
    let path = directory.path().join("runtime-release.json");
    cache.store(&path, &trust, 1).unwrap();
    let loaded = CachedRuntimeRelease::load(&path, &trust, 10).unwrap();
    assert_eq!(loaded.release(), cache.release());
    assert_eq!(loaded.verified_at_unix_seconds(), 1);
}

#[test]
fn cache_rechecks_expiry_and_signer_revocation() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let trust = trust(&key);
    let cache = CachedRuntimeRelease::verify(release(&key, 10), &trust, 1).unwrap();
    let directory = tempdir().unwrap();
    let path = directory.path().join("runtime-release.json");
    cache.store(&path, &trust, 1).unwrap();
    assert!(CachedRuntimeRelease::load(&path, &trust, 11).is_err());
    let mut revoked = trust;
    revoked.revoke_signer("release-1");
    assert!(CachedRuntimeRelease::load(&path, &revoked, 1).is_err());
}

#[test]
fn cache_rejects_corrupted_json() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let trust = trust(&key);
    let directory = tempdir().unwrap();
    let path = directory.path().join("runtime-release.json");
    std::fs::write(&path, b"not json").unwrap();
    assert!(CachedRuntimeRelease::load(&path, &trust, 1).is_err());
}

#[cfg(unix)]
#[test]
fn cache_refuses_a_symlinked_parent_directory() {
    let key = SigningKey::from_bytes(&[3; 32]);
    let trust = trust(&key);
    let cache = CachedRuntimeRelease::verify(release(&key, 10), &trust, 1).unwrap();
    let directory = tempdir().unwrap();
    let destination = directory.path().join("destination");
    std::fs::create_dir(&destination).unwrap();
    std::os::unix::fs::symlink(&destination, directory.path().join("link")).unwrap();
    assert!(
        cache
            .store(&directory.path().join("link/cache.json"), &trust, 1)
            .is_err()
    );
}
