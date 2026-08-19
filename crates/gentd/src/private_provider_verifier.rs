//! Prefix-only executable verification for dormant Gent-managed npm provisioning.
//!
//! This component never searches `PATH`, invokes an interactive provider command, or composes
//! observer authority. The only child process it can create is the fixed `--version` probe.

use std::path::{Path, PathBuf};

use gent_drivers::{
    discovery::{ProbeError, VersionProbe},
    lock::capture,
};
use gent_protocol::DependencyProvider;
use gent_types::ProvisionedProviderLock;

use crate::{
    private_provider_provisioning::ProvisionedProviderVerifier,
    provider_resolver::SystemVersionProbe,
};

const VERSION_ARGUMENT: &str = "--version";

/// Verifies only the canonical `bin/claude` or `bin/codex` executable below one Gent prefix.
#[derive(Clone, Debug)]
pub(crate) struct PrivatePrefixProvisionedProviderVerifier<P = SystemVersionProbe> {
    probe: P,
    compatibility_entry: String,
}

impl PrivatePrefixProvisionedProviderVerifier<SystemVersionProbe> {
    /// Creates the production verifier with the daemon's fixed, bounded version probe.
    #[must_use]
    pub(crate) fn system(compatibility_entry: impl Into<String>) -> Self {
        Self::new(SystemVersionProbe, compatibility_entry)
    }
}

impl<P> PrivatePrefixProvisionedProviderVerifier<P> {
    /// Binds a version-probe port and a nonempty provenance label without discovering a binary.
    #[must_use]
    pub(crate) fn new(probe: P, compatibility_entry: impl Into<String>) -> Self {
        Self {
            probe,
            compatibility_entry: compatibility_entry.into(),
        }
    }
}

impl<P> ProvisionedProviderVerifier for PrivatePrefixProvisionedProviderVerifier<P>
where
    P: Clone + Send + Sync + VersionProbe,
{
    fn lock(
        &self,
        provider: DependencyProvider,
        prefix: &Path,
    ) -> Result<ProvisionedProviderLock, String> {
        let prefix = canonical_prefix(prefix)?;
        let executable = private_executable(provider, &prefix)?;
        let before = capture(
            provider.as_str(),
            &executable,
            "unprobed",
            &self.compatibility_entry,
        )
        .map_err(|_| "private provider executable cannot be identity-locked".to_owned())?;
        let version = self
            .probe
            .probe(&executable, VERSION_ARGUMENT)
            .map_err(probe_error)?;
        valid_version(&version)?;
        let run_lock = capture(
            provider.as_str(),
            &executable,
            &version,
            &self.compatibility_entry,
        )
        .map_err(|_| "private provider executable cannot be identity-locked".to_owned())?;
        (before.canonical_path == run_lock.canonical_path
            && before.file_identity == run_lock.file_identity
            && before.digest_sha256 == run_lock.digest_sha256)
            .then_some(ProvisionedProviderLock { run_lock })
            .ok_or_else(|| "private provider executable changed during version probe".to_owned())
    }
}

fn canonical_prefix(prefix: &Path) -> Result<PathBuf, String> {
    let prefix = prefix
        .canonicalize()
        .map_err(|_| "private npm prefix is unavailable".to_owned())?;
    prefix
        .is_dir()
        .then_some(prefix)
        .ok_or_else(|| "private npm prefix is not a directory".to_owned())
}

fn private_executable(provider: DependencyProvider, prefix: &Path) -> Result<PathBuf, String> {
    let candidate = prefix.join("bin").join(binary_name(provider));
    let executable = candidate
        .canonicalize()
        .map_err(|_| "private provider executable is unavailable".to_owned())?;
    (executable.starts_with(prefix) && executable.is_file())
        .then_some(executable)
        .ok_or_else(|| "private provider executable escapes Gent prefix".to_owned())
}

#[cfg(windows)]
fn binary_name(provider: DependencyProvider) -> String {
    format!("{}.cmd", provider.as_str())
}

#[cfg(not(windows))]
fn binary_name(provider: DependencyProvider) -> String {
    provider.as_str().into()
}

fn valid_version(version: &str) -> Result<(), String> {
    (!version.is_empty()
        && version.len() <= 512
        && version.trim() == version
        && !version.contains('\0'))
    .then_some(())
    .ok_or_else(|| "private provider version output is invalid".to_owned())
}

fn probe_error(_: ProbeError) -> String {
    "private provider version probe failed".into()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use gent_drivers::discovery::{ProbeError, VersionProbe};
    use gent_protocol::DependencyProvider;

    use super::{PrivatePrefixProvisionedProviderVerifier, ProvisionedProviderVerifier};

    #[derive(Clone)]
    struct Probe {
        version: Result<String, &'static str>,
        replace_binary: bool,
    }

    impl VersionProbe for Probe {
        fn probe(&self, executable: &Path, argument: &str) -> Result<String, ProbeError> {
            assert_eq!(argument, "--version");
            assert!(executable.ends_with("codex"));
            if self.replace_binary {
                fs::write(executable, "changed provider").unwrap();
            }
            self.version
                .clone()
                .map_err(|error| ProbeError::Failed(error.into()))
        }
    }

    #[test]
    fn locks_only_the_private_codex_executable_with_probe_and_identity() {
        let root = tempfile::tempdir().unwrap();
        let prefix = prefix(root.path());
        let lock = verifier(Ok("1.2.3".into()))
            .lock(DependencyProvider::Codex, &prefix)
            .unwrap();
        assert_eq!(lock.run_lock.provider, "codex");
        assert_eq!(lock.run_lock.version, "1.2.3");
        assert_eq!(lock.run_lock.digest_sha256.len(), 64);
        assert!(
            lock.run_lock
                .canonical_path
                .starts_with(&prefix.canonicalize().unwrap().display().to_string())
        );
    }

    #[test]
    fn provider_binaries_are_fixed_to_claude_or_codex_under_the_private_prefix() {
        assert_eq!(
            super::binary_name(DependencyProvider::Claude),
            expected("claude")
        );
        assert_eq!(
            super::binary_name(DependencyProvider::Codex),
            expected("codex")
        );
    }

    #[test]
    fn rejects_prefix_escape_and_invalid_version_output() {
        let root = tempfile::tempdir().unwrap();
        let escaped_prefix = root.path().join("npm-global");
        fs::create_dir_all(escaped_prefix.join("bin")).unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, "outside").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, escaped_prefix.join("bin/codex")).unwrap();
        #[cfg(not(unix))]
        fs::write(escaped_prefix.join("bin/codex"), "outside").unwrap();
        assert!(
            verifier(Ok("1.2.3".into()))
                .lock(DependencyProvider::Codex, &escaped_prefix)
                .is_err()
        );

        let fresh = tempfile::tempdir().unwrap();
        let prefix = prefix(fresh.path());
        assert!(
            verifier(Err("failed"))
                .lock(DependencyProvider::Codex, &prefix)
                .is_err()
        );
        assert!(
            verifier(Ok(" 1.2.3".into()))
                .lock(DependencyProvider::Codex, &prefix)
                .is_err()
        );
    }

    #[test]
    fn refuses_a_binary_that_changes_during_the_fixed_version_probe() {
        let root = tempfile::tempdir().unwrap();
        assert!(
            verifier_with_mutation(Ok("1.2.3".into()))
                .lock(DependencyProvider::Codex, &prefix(root.path()))
                .is_err()
        );
    }

    fn verifier(
        version: Result<String, &'static str>,
    ) -> PrivatePrefixProvisionedProviderVerifier<Probe> {
        PrivatePrefixProvisionedProviderVerifier::new(
            Probe {
                version,
                replace_binary: false,
            },
            "provisioned",
        )
    }

    fn verifier_with_mutation(
        version: Result<String, &'static str>,
    ) -> PrivatePrefixProvisionedProviderVerifier<Probe> {
        PrivatePrefixProvisionedProviderVerifier::new(
            Probe {
                version,
                replace_binary: true,
            },
            "provisioned",
        )
    }

    fn prefix(root: &Path) -> std::path::PathBuf {
        let prefix = root.join("npm-global");
        let bin = prefix.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("codex"), "codex provider").unwrap();
        prefix
    }

    #[cfg(windows)]
    fn expected(name: &str) -> String {
        format!("{name}.cmd")
    }

    #[cfg(not(windows))]
    fn expected(name: &str) -> String {
        name.into()
    }
}
