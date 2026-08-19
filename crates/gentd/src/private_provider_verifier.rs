//! Prefix-only executable verification for dormant Gent-managed npm provisioning.
//!
//! This component never searches `PATH`, invokes an interactive provider command, or composes
//! observer authority. The only child process it can create is the fixed `--version` probe.

use std::path::{Path, PathBuf};

use gent_drivers::{lock::capture, node_runtime_lock::NodeRuntimeLock};
use gent_protocol::DependencyProvider;
use gent_types::ProvisionedProviderLock;

use crate::private_provider_provisioning::ProvisionedProviderVerifier;

#[path = "private_provider_verifier_probe.rs"]
mod probe;
use probe::{LockedNodeVersionProbe, PrivateVersionProbe};

const VERSION_ARGUMENT: &str = "--version";

/// Verifies only the canonical `bin/claude` or `bin/codex` executable below one Gent prefix.
#[derive(Clone, Debug)]
pub(crate) struct PrivatePrefixProvisionedProviderVerifier<P = LockedNodeVersionProbe> {
    probe: P,
}

impl PrivatePrefixProvisionedProviderVerifier<LockedNodeVersionProbe> {
    /// Creates the production verifier through the exact app Node runtime.
    #[must_use]
    pub(crate) fn system(runtime: NodeRuntimeLock) -> Self {
        Self::new(LockedNodeVersionProbe::new(runtime))
    }
}

impl<P> PrivatePrefixProvisionedProviderVerifier<P> {
    /// Binds a version-probe port without discovering a binary.
    #[must_use]
    pub(crate) fn new(probe: P) -> Self {
        Self { probe }
    }
}

impl<P> ProvisionedProviderVerifier for PrivatePrefixProvisionedProviderVerifier<P>
where
    P: PrivateVersionProbe,
{
    fn lock(
        &self,
        provider: DependencyProvider,
        prefix: &Path,
    ) -> Result<ProvisionedProviderLock, String> {
        let prefix = canonical_prefix(prefix)?;
        let executable = private_executable(provider, &prefix)?;
        let before = capture(provider.as_str(), &executable, "unprobed", "unbound")
            .map_err(|_| "private provider executable cannot be identity-locked".to_owned())?;
        let version = self
            .probe
            .probe(provider, &before, &executable, VERSION_ARGUMENT)
            .map_err(probe_error)?;
        valid_version(&version)?;
        let run_lock = capture(provider.as_str(), &executable, &version, "unbound")
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

fn probe_error(_: String) -> String {
    "private provider version probe failed".into()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use gent_protocol::DependencyProvider;
    use gent_types::RunVersionLock;

    use super::{
        PrivatePrefixProvisionedProviderVerifier, ProvisionedProviderVerifier,
        probe::PrivateVersionProbe,
    };

    #[derive(Clone)]
    struct Probe {
        version: Result<String, &'static str>,
        replace_binary: bool,
    }

    impl PrivateVersionProbe for Probe {
        fn probe(
            &self,
            _: DependencyProvider,
            _: &RunVersionLock,
            executable: &Path,
            argument: &str,
        ) -> Result<String, String> {
            assert_eq!(argument, "--version");
            assert!(executable.ends_with("codex"));
            if self.replace_binary {
                fs::write(executable, "changed provider").unwrap();
            }
            self.version.clone().map_err(Into::into)
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
        assert_eq!(lock.run_lock.compatibility_entry, "unbound");
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

    #[cfg(unix)]
    #[test]
    fn production_probe_resolves_a_provider_shim_through_only_locked_node() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let runtime_root = root.path().join("node");
        let bin = runtime_root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let node = bin.join("node");
        fs::write(&node, "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(bin.join("npm"), "#!/bin/sh\nexit 0\n").unwrap();
        for path in [&node, &bin.join("npm")] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let npm_cli = runtime_root.join("lib/node_modules/npm/bin");
        fs::create_dir_all(&npm_cli).unwrap();
        fs::write(npm_cli.join("npm-cli.js"), "npm cli").unwrap();
        let prefix = root.path().join("npm-global");
        fs::create_dir_all(prefix.join("bin")).unwrap();
        let provider = prefix.join("bin/codex");
        fs::write(&provider, "#!/bin/sh\ncommand -v node\n").unwrap();
        fs::set_permissions(&provider, fs::Permissions::from_mode(0o700)).unwrap();

        let verifier = PrivatePrefixProvisionedProviderVerifier::system(
            gent_drivers::node_runtime_lock::NodeRuntimeLock::capture(&node).unwrap(),
        );
        assert_eq!(
            verifier
                .lock(DependencyProvider::Codex, &prefix)
                .unwrap()
                .run_lock
                .version,
            node.canonicalize().unwrap().display().to_string()
        );
    }

    fn verifier(
        version: Result<String, &'static str>,
    ) -> PrivatePrefixProvisionedProviderVerifier<Probe> {
        PrivatePrefixProvisionedProviderVerifier::new(Probe {
            version,
            replace_binary: false,
        })
    }

    fn verifier_with_mutation(
        version: Result<String, &'static str>,
    ) -> PrivatePrefixProvisionedProviderVerifier<Probe> {
        PrivatePrefixProvisionedProviderVerifier::new(Probe {
            version,
            replace_binary: true,
        })
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
