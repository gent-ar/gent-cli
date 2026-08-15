//! Read-only compatibility assessment for `gent doctor`.

use gent_adapters::compatibility::TrustedKeySet;
use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
use gent_types::{CompatibilityTrust, ExecutableIdentity, RunVersionLock};

/// A verified cache and trusted key set injected by the daemon composition root.
#[derive(Clone, Debug, Default)]
pub(crate) struct CompatibilityAssessment {
    keys: TrustedKeySet,
    cached: Option<CachedCompatibilityManifest>,
    now: u64,
}

impl CompatibilityAssessment {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn configured(
        keys: TrustedKeySet,
        cached: CachedCompatibilityManifest,
        now: u64,
    ) -> Self {
        Self {
            keys,
            cached: Some(cached),
            now,
        }
    }

    pub(crate) fn assess(
        &self,
        provider: &str,
        identity: &ExecutableIdentity,
    ) -> CompatibilityTrust {
        let Some(cached) = &self.cached else {
            return CompatibilityTrust::NotConfigured;
        };
        let Some(version) = &identity.version else {
            return CompatibilityTrust::Untrusted;
        };
        let Some(entry) = cached.manifest.payload.entries.iter().find(|entry| {
            entry.provider == provider && entry.version == *version && !entry.revoked
        }) else {
            return CompatibilityTrust::Untrusted;
        };
        let lock = RunVersionLock {
            provider: provider.into(),
            canonical_path: identity.canonical_path.clone(),
            file_identity: identity.file_identity.clone(),
            digest_sha256: identity.digest_sha256.clone(),
            version: version.clone(),
            compatibility_entry: entry.id.clone(),
        };
        if cached.revalidate(&self.keys, self.now).is_ok()
            && self
                .keys
                .verify_lock(&cached.manifest, &lock, self.now)
                .is_ok()
        {
            CompatibilityTrust::Verified
        } else {
            CompatibilityTrust::Untrusted
        }
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use gent_adapters::compatibility::{
        CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest,
    };
    use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
    use gent_types::{CompatibilityTrust, ExecutableIdentity};

    use super::{CompatibilityAssessment, TrustedKeySet};

    fn assessment(expires_at: u64, revoked: bool) -> CompatibilityAssessment {
        let key = SigningKey::from_bytes(&[3; 32]);
        let payload = CompatibilityManifest {
            manifest_version: 1,
            expires_at_unix_seconds: expires_at,
            entries: vec![CompatibilityEntry {
                id: "claude-1".into(),
                provider: "claude".into(),
                version: "1.0".into(),
                revoked,
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

    fn identity(version: Option<&str>) -> ExecutableIdentity {
        ExecutableIdentity {
            canonical_path: "/public/claude".into(),
            file_identity: "1:2".into(),
            digest_sha256: "digest".into(),
            version: version.map(str::to_owned),
        }
    }

    #[test]
    fn verifies_only_an_active_matching_signed_entry() {
        assert_eq!(
            assessment(20, false).assess("claude", &identity(Some("1.0"))),
            CompatibilityTrust::Verified
        );
        assert_eq!(
            assessment(20, false).assess("claude", &identity(Some("2.0"))),
            CompatibilityTrust::Untrusted
        );
        assert_eq!(
            assessment(20, true).assess("claude", &identity(Some("1.0"))),
            CompatibilityTrust::Untrusted
        );
        assert_eq!(
            assessment(9, false).assess("claude", &identity(Some("1.0"))),
            CompatibilityTrust::Untrusted
        );
    }

    #[test]
    fn missing_configuration_or_version_is_not_verified() {
        assert_eq!(
            CompatibilityAssessment::default().assess("claude", &identity(Some("1.0"))),
            CompatibilityTrust::NotConfigured
        );
        assert_eq!(
            assessment(20, false).assess("claude", &identity(None)),
            CompatibilityTrust::Untrusted
        );
    }
}
