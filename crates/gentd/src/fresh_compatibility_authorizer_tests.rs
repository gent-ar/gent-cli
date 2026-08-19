use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use ed25519_dalek::{Signer, SigningKey};
use gent_adapters::compatibility::{
    CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet,
};
use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
use gent_ports::RunVersionAuthorizer;
use gent_types::RunVersionLock;

use super::FreshCompatibilityAuthorizer;
use crate::authority_clock::AuthorityClock;
use crate::compatibility_assessment::CompatibilityAssessment;

#[derive(Clone, Debug)]
struct Clock(Arc<AtomicU64>);

impl AuthorityClock for Clock {
    fn now_unix_seconds(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[test]
fn reauthorizes_a_durable_lock_at_each_effect_boundary() {
    let clock = Clock(Arc::new(AtomicU64::new(9)));
    let authorizer = FreshCompatibilityAuthorizer::new(assessment(10), clock.clone());

    assert!(authorizer.authorize(&lock()).is_ok());
    clock.0.store(11, Ordering::SeqCst);
    assert!(authorizer.authorize(&lock()).is_err());
}

fn assessment(expires_at: u64) -> CompatibilityAssessment {
    let key = SigningKey::from_bytes(&[8; 32]);
    let payload = CompatibilityManifest {
        manifest_version: 1,
        expires_at_unix_seconds: expires_at,
        entries: vec![CompatibilityEntry {
            id: "codex-1".into(),
            provider: "codex".into(),
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
    CompatibilityAssessment::configured(keys, cached, 1)
}

fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "codex".into(),
        canonical_path: "/private/codex".into(),
        file_identity: "1:2".into(),
        digest_sha256: "digest".into(),
        version: "1.0".into(),
        compatibility_entry: "codex-1".into(),
    }
}
