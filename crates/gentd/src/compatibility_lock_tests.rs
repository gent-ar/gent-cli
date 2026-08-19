use ed25519_dalek::{Signer, SigningKey};
use gent_adapters::compatibility::{
    CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet,
};
use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
use gent_ports::RunVersionAuthorizer;
use gent_types::RunVersionLock;

use crate::compatibility_assessment::CompatibilityAssessment;

fn assessment() -> CompatibilityAssessment {
    let key = SigningKey::from_bytes(&[3; 32]);
    let payload = CompatibilityManifest {
        manifest_version: 1,
        expires_at_unix_seconds: 20,
        entries: vec![CompatibilityEntry {
            id: "claude-1".into(),
            provider: "claude".into(),
            version: "1.0".into(),
            digest_sha256: "digest".into(),
            revoked: false,
        }],
    };
    let manifest = SignedCompatibilityManifest {
        key_id: "test".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    };
    let mut keys = TrustedKeySet::default();
    keys.trust("test", key.verifying_key());
    let cached = CachedCompatibilityManifest::verify(manifest, &keys, 1).unwrap();
    CompatibilityAssessment::configured(keys, cached, 10)
}

fn lock(entry: &str, digest: &str) -> RunVersionLock {
    RunVersionLock {
        provider: "claude".into(),
        canonical_path: "/public/claude".into(),
        file_identity: "1:2".into(),
        digest_sha256: digest.into(),
        version: "1.0".into(),
        compatibility_entry: entry.into(),
    }
}

#[test]
fn authorization_requires_an_active_digest_bound_signed_entry() {
    assert!(
        CompatibilityAssessment::default()
            .authorize(&lock("claude-1", "digest"))
            .is_err()
    );
    assert!(assessment().authorize(&lock("claude-1", "digest")).is_ok());
    assert!(
        assessment()
            .authorize(&lock("claude-1", "changed"))
            .is_err()
    );
    assert!(assessment().authorize(&lock("other", "digest")).is_err());
}

#[test]
fn observed_locks_bind_only_to_the_matching_signed_entry() {
    let bound = assessment()
        .bind_observed_lock(lock("unbound", "digest"))
        .unwrap();
    assert_eq!(bound.compatibility_entry, "claude-1");
    assert!(
        assessment()
            .bind_observed_lock(lock("unbound", "different"))
            .is_err()
    );
}

#[test]
fn explicit_authority_time_refuses_a_lock_after_manifest_expiry() {
    let bound = assessment()
        .bind_observed_lock_at(lock("unbound", "digest"), 20)
        .unwrap();

    assert!(assessment().authorize_at(&bound, 21).is_err());
    assert!(
        assessment()
            .bind_observed_lock_at(lock("unbound", "digest"), 21)
            .is_err()
    );
}
