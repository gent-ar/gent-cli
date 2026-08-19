//! Read-only compatibility assessment for `gent doctor`.

use std::path::Path;

use ed25519_dalek::VerifyingKey;
use gent_adapters::compatibility::TrustedKeySet;
use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
use gent_ports::{PublicProviderRunError, RunVersionAuthorizer};
use gent_types::{CompatibilityTrust, ExecutableIdentity, RunVersionLock};
use sha2::{Digest, Sha256};

use crate::private_provider_compatibility::ProvisionedProviderCompatibility;

/// A verified cache and trusted key set injected by the daemon composition root.
#[derive(Clone, Debug, Default)]
pub(crate) struct CompatibilityAssessment {
    keys: TrustedKeySet,
    cached: Option<CachedCompatibilityManifest>,
    now: u64,
    configured: bool,
}

impl CompatibilityAssessment {
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
            configured: true,
        }
    }

    /// Loads only local, previously verified cache state. Invalid configuration is untrusted.
    #[must_use]
    pub(crate) fn load(path: Option<&Path>, key_specs: &[String], now: u64) -> Self {
        if path.is_none() && key_specs.is_empty() {
            return Self::default();
        }
        let mut keys = TrustedKeySet::default();
        if key_specs
            .iter()
            .any(|spec| add_key(&mut keys, spec).is_err())
        {
            return Self::untrusted(now);
        }
        let Some(path) = path else {
            return Self::untrusted(now);
        };
        match CachedCompatibilityManifest::load(path, &keys, now) {
            Ok(cached) => Self::configured(keys, cached, now),
            Err(_) => Self::untrusted(now),
        }
    }

    pub(crate) fn assess(
        &self,
        provider: &str,
        identity: &ExecutableIdentity,
    ) -> CompatibilityTrust {
        if self.cached.is_none() {
            return if self.configured {
                CompatibilityTrust::Untrusted
            } else {
                CompatibilityTrust::NotConfigured
            };
        }
        let Some(version) = &identity.version else {
            return CompatibilityTrust::Untrusted;
        };
        let lock = RunVersionLock {
            provider: provider.into(),
            canonical_path: identity.canonical_path.clone(),
            file_identity: identity.file_identity.clone(),
            digest_sha256: identity.digest_sha256.clone(),
            version: version.clone(),
            compatibility_entry: "unbound".into(),
        };
        if self.bind_observed_lock(lock).is_ok() {
            CompatibilityTrust::Verified
        } else {
            CompatibilityTrust::Untrusted
        }
    }

    /// Returns the digest of the exact signed compatibility envelope currently in use.
    ///
    /// A future authority composition binds its separately approved profile to this value before
    /// an injected resolver is allowed to inspect an executable. The digest intentionally covers
    /// the signer id and signature as well as the payload, not a lossy projection of entries.
    #[must_use]
    pub(crate) fn manifest_sha256(&self) -> Option<String> {
        let manifest = &self.cached.as_ref()?.manifest;
        let bytes = serde_json::to_vec(manifest).ok()?;
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Some(format!("{:x}", hasher.finalize()))
    }

    pub(crate) fn remediation(
        present: bool,
        trust: &CompatibilityTrust,
        missing_remediation: &str,
    ) -> String {
        if !present {
            return missing_remediation.into();
        }
        match trust {
            CompatibilityTrust::Verified => {
                "Signed compatibility evidence matches this executable.".into()
            }
            CompatibilityTrust::Untrusted => {
                "Configured signed compatibility evidence does not trust this executable.".into()
            }
            CompatibilityTrust::NotConfigured => {
                "A public executable was observed, but no signed compatibility manifest is configured."
                    .into()
            }
        }
    }

    /// Binds a daemon-observed executable lock to its exact signed compatibility entry.
    ///
    /// # Errors
    /// Returns a controlled denial when no current digest-bound entry authorizes the lock.
    pub fn bind_observed_lock(
        &self,
        lock: RunVersionLock,
    ) -> Result<RunVersionLock, PublicProviderRunError> {
        self.bind_observed_lock_at(lock, self.now)
    }

    /// Binds an observed lock against the signed manifest at the supplied authority time.
    ///
    /// Long-lived authorities call this at their effect boundary rather than reusing bootstrap
    /// time, so expiry and signer revocation cannot remain trusted after a daemon stays alive.
    ///
    /// # Errors
    /// Returns a controlled denial when no current digest-bound entry authorizes the lock.
    pub(crate) fn bind_observed_lock_at(
        &self,
        mut lock: RunVersionLock,
        now_unix_seconds: u64,
    ) -> Result<RunVersionLock, PublicProviderRunError> {
        let cached = self
            .cached
            .as_ref()
            .ok_or(PublicProviderRunError::CompatibilityDenied)?;
        cached
            .revalidate(&self.keys, now_unix_seconds)
            .map_err(|_| PublicProviderRunError::CompatibilityDenied)?;
        let entry = cached
            .manifest
            .payload
            .entries
            .iter()
            .find(|entry| {
                entry.provider == lock.provider
                    && entry.version == lock.version
                    && entry.digest_sha256 == lock.digest_sha256
                    && !entry.revoked
            })
            .ok_or(PublicProviderRunError::CompatibilityDenied)?;
        lock.compatibility_entry = entry.id.clone();
        self.authorize_at(&lock, now_unix_seconds)?;
        Ok(lock)
    }

    /// Reauthorizes one exact lock at the supplied authority time.
    ///
    /// # Errors
    /// Returns a controlled denial for expired, revoked, or mismatched compatibility evidence.
    pub(crate) fn authorize_at(
        &self,
        lock: &RunVersionLock,
        now_unix_seconds: u64,
    ) -> Result<(), PublicProviderRunError> {
        let cached = self
            .cached
            .as_ref()
            .ok_or(PublicProviderRunError::CompatibilityDenied)?;
        cached
            .revalidate(&self.keys, now_unix_seconds)
            .and_then(|()| {
                self.keys
                    .verify_lock(&cached.manifest, lock, now_unix_seconds)
            })
            .map_err(|_| PublicProviderRunError::CompatibilityDenied)
    }
}

impl RunVersionAuthorizer for CompatibilityAssessment {
    fn authorize(&self, lock: &RunVersionLock) -> Result<(), PublicProviderRunError> {
        self.authorize_at(lock, self.now)
    }
}

impl ProvisionedProviderCompatibility for CompatibilityAssessment {
    fn bind(
        &self,
        observed: RunVersionLock,
        now_unix_seconds: u64,
    ) -> Result<RunVersionLock, String> {
        self.bind_observed_lock_at(observed, now_unix_seconds)
            .map_err(|_| {
                "provider executable is not authorized by current compatibility evidence".into()
            })
    }
}

impl CompatibilityAssessment {
    fn untrusted(now: u64) -> Self {
        Self {
            keys: TrustedKeySet::default(),
            cached: None,
            now,
            configured: true,
        }
    }
}

fn add_key(keys: &mut TrustedKeySet, spec: &str) -> Result<(), ()> {
    let Some((id, encoded)) = spec.split_once(':') else {
        return Err(());
    };
    let bytes = hex::decode(encoded).map_err(|_| ())?;
    let key =
        VerifyingKey::from_bytes(bytes.as_slice().try_into().map_err(|_| ())?).map_err(|_| ())?;
    keys.trust(id, key);
    Ok(())
}

#[cfg(test)]
#[path = "compatibility_assessment_tests.rs"]
mod tests;
