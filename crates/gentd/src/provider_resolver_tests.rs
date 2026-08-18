use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer, SigningKey};
use gent_adapters::compatibility::{
    CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet,
};
use gent_adapters::compatibility_cache::CachedCompatibilityManifest;
use gent_drivers::discovery::{DiscoveryError, ExecutableDiscovery, ProbeError, VersionProbe};
use gent_drivers::lock::capture;
use gent_ports::{PublicProviderResolver, PublicProviderRunError};

use crate::compatibility_assessment::CompatibilityAssessment;
use crate::provider_resolver::{
    CodexOnlyResolver, DaemonProviderResolver, PrivatePrefixDiscovery, PrivatePrefixFirstDiscovery,
};

#[derive(Clone, Debug)]
struct Found(PathBuf);

impl ExecutableDiscovery for Found {
    fn find(&self, _: &str) -> Result<Option<PathBuf>, DiscoveryError> {
        Ok(Some(self.0.clone()))
    }
}

#[derive(Clone, Debug)]
struct Version(&'static str);

impl VersionProbe for Version {
    fn probe(&self, _: &Path, argument: &str) -> Result<String, ProbeError> {
        assert_eq!(argument, "--version");
        Ok(self.0.into())
    }
}

fn assessment(provider: &str, path: &Path, version: &str) -> CompatibilityAssessment {
    let observed = capture(provider, path, version, "unbound").unwrap();
    let key = SigningKey::from_bytes(&[8; 32]);
    let payload = CompatibilityManifest {
        manifest_version: 1,
        expires_at_unix_seconds: 20,
        entries: vec![CompatibilityEntry {
            id: format!("{provider}-test"),
            provider: provider.into(),
            version: version.into(),
            digest_sha256: observed.digest_sha256,
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

#[test]
fn resolver_captures_and_binds_only_daemon_observed_identity() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    std::fs::write(&executable, "public provider binary").unwrap();
    let resolver = DaemonProviderResolver::new(
        assessment("claude", &executable, "1.0"),
        Found(executable.clone()),
        Version("1.0"),
    );
    let lock = resolver.resolve("claude").unwrap();
    assert_eq!(lock.compatibility_entry, "claude-test");
    assert_eq!(lock.version, "1.0");
    assert_eq!(
        lock.canonical_path,
        std::fs::canonicalize(executable)
            .unwrap()
            .display()
            .to_string()
    );
}

#[test]
fn resolver_rejects_private_or_changed_provider_identity() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("claude");
    std::fs::write(&executable, "before").unwrap();
    let resolver = DaemonProviderResolver::new(
        assessment("claude", &executable, "1.0"),
        Found(executable.clone()),
        Version("1.0"),
    );
    assert_eq!(
        resolver.resolve("claurst"),
        Err(PublicProviderRunError::CompatibilityDenied)
    );
    std::fs::write(executable, "after").unwrap();
    assert_eq!(
        resolver.resolve("claude"),
        Err(PublicProviderRunError::CompatibilityDenied)
    );
}

#[derive(Debug)]
struct NeverDiscovery;

impl ExecutableDiscovery for NeverDiscovery {
    fn find(&self, _: &str) -> Result<Option<PathBuf>, DiscoveryError> {
        panic!("a non-Codex resolver request must not discover an executable")
    }
}

#[derive(Debug)]
struct NeverProbe;

impl VersionProbe for NeverProbe {
    fn probe(&self, _: &Path, _: &str) -> Result<String, ProbeError> {
        panic!("a non-Codex resolver request must not probe an executable")
    }
}

#[test]
fn codex_only_resolver_denies_other_providers_before_discovery_or_probe() {
    let resolver = CodexOnlyResolver::new(DaemonProviderResolver::new(
        CompatibilityAssessment::default(),
        NeverDiscovery,
        NeverProbe,
    ));
    for provider in ["claude", "claurst", "gent", ""] {
        assert_eq!(
            resolver.resolve(provider),
            Err(PublicProviderRunError::CompatibilityDenied)
        );
    }
}

#[test]
fn codex_only_resolver_delegates_a_locked_codex_lookup() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("codex");
    std::fs::write(&executable, "private Codex binary").unwrap();
    let resolver = CodexOnlyResolver::new(DaemonProviderResolver::new(
        assessment("codex", &executable, "0.147.0"),
        Found(executable),
        Version("0.147.0"),
    ));
    let lock = resolver.resolve("codex").unwrap();
    assert_eq!(lock.provider, "codex");
    assert_eq!(lock.compatibility_entry, "codex-test");
}

#[test]
fn private_prefix_discovery_precedes_its_injected_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("npm-global");
    let bin = prefix.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let installed = bin.join("codex");
    std::fs::write(&installed, "private provider binary").unwrap();
    let fallback = directory.path().join("fallback-codex");
    std::fs::write(&fallback, "fallback provider binary").unwrap();
    let discovery = PrivatePrefixFirstDiscovery::new(prefix, Found(fallback));
    assert_eq!(
        discovery.find("codex").unwrap(),
        Some(installed),
        "Gent-owned npm installation must win over fallback discovery"
    );
}

#[test]
fn private_prefix_authority_discovery_never_uses_path_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let prefix = directory.path().join("npm-global");
    let discovery = PrivatePrefixDiscovery::new(prefix.clone());
    assert_eq!(discovery.find("codex").unwrap(), None);

    let bin = prefix.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let installed = bin.join(binary_name());
    std::fs::write(&installed, "private provider binary").unwrap();
    assert_eq!(discovery.find("codex").unwrap(), Some(installed));
}

#[cfg(windows)]
fn binary_name() -> &'static str {
    "codex.cmd"
}

#[cfg(not(windows))]
fn binary_name() -> &'static str {
    "codex"
}
