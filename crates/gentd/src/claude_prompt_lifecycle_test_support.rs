use ed25519_dalek::{Signer, SigningKey};
use gent_adapters::compatibility::{
    CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet,
};
use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
use gent_types::RunVersionLock;

use crate::compatibility_assessment::CompatibilityAssessment;

pub(crate) fn lock() -> RunVersionLock {
    RunVersionLock {
        provider: "claude".into(),
        canonical_path: "/verified/claude".into(),
        file_identity: "1:2".into(),
        digest_sha256: "b".repeat(64),
        version: "2.1.0".into(),
        compatibility_entry: "claude-2.1.0".into(),
    }
}
pub(crate) fn compatibility() -> CompatibilityAssessment {
    let key = SigningKey::from_bytes(&[8; 32]);
    let payload = CompatibilityManifest {
        manifest_version: 1,
        expires_at_unix_seconds: 20,
        entries: vec![CompatibilityEntry {
            id: "claude-2.1.0".into(),
            provider: "claude".into(),
            version: "2.1.0".into(),
            digest_sha256: "b".repeat(64),
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
    CompatibilityAssessment::configured(
        keys.clone(),
        CachedCompatibilityManifest::verify(manifest, &keys, 1).unwrap(),
        10,
    )
}
