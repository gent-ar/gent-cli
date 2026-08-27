//! Daemon-owned app Node/npm runtime lock for future approved provisioning.
//!
//! The shipped observer never constructs this value. An approved host must
//! recheck it immediately before any package operation.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

use gent_drivers::{
    NodeReadOnlyHostLauncher,
    installer::NpmGlobalPrefix,
    node_runtime_lock::{NodeRuntimeLock, NodeRuntimeLockError},
};

const NODE_BINARY_ENV: &str = "GENT_NODE_BINARY";

/// Immutable app runtime identity plus Gent's private installation prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AppNodeRuntimeLock {
    lock: NodeRuntimeLock,
    private_prefix: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppNodeRuntimeLockError {
    #[error("Node runtime is unavailable; set GENT_NODE_BINARY or install Gent's packaged runtime")]
    MissingNode,
    #[error(transparent)]
    Runtime(#[from] NodeRuntimeLockError),
}

impl AppNodeRuntimeLock {
    /// Captures the app-supplied Node/npm identity using `GENT_NODE_BINARY`.
    ///
    /// # Errors
    /// Returns an error when the app runtime is absent or cannot be locked.
    pub(crate) fn from_environment(data_dir: &Path) -> Result<Self, AppNodeRuntimeLockError> {
        Self::capture(env::var_os(NODE_BINARY_ENV), data_dir)
    }

    pub(crate) fn from_standalone_environment(
        data_dir: &Path,
    ) -> Result<Self, AppNodeRuntimeLockError> {
        Self::capture_standalone(env::var_os(NODE_BINARY_ENV), data_dir)
    }

    /// Captures a caller-supplied app runtime for a private, uncomposed authority seam.
    ///
    /// # Errors
    /// Returns an error when the supplied runtime cannot be locked.
    pub(crate) fn capture(
        node: Option<OsString>,
        data_dir: &Path,
    ) -> Result<Self, AppNodeRuntimeLockError> {
        let node = node.ok_or(AppNodeRuntimeLockError::MissingNode)?;
        Ok(Self {
            lock: NodeRuntimeLock::capture(Path::new(&node))?,
            private_prefix: data_dir.join("providers").join("npm-global"),
        })
    }

    pub(crate) fn capture_standalone(
        node: Option<OsString>,
        data_dir: &Path,
    ) -> Result<Self, AppNodeRuntimeLockError> {
        let node = node.map_or_else(|| packaged_node(data_dir), PathBuf::from);
        Ok(Self {
            lock: NodeRuntimeLock::capture(&node)?,
            private_prefix: data_dir.join("providers").join("npm-global"),
        })
    }

    /// Rechecks the exact Node/npm pair before a future approved operation.
    pub(crate) fn recheck(&self) -> Result<(), AppNodeRuntimeLockError> {
        self.lock.recheck().map_err(Into::into)
    }

    /// The digest consumed by signed package-policy verification.
    #[must_use]
    pub(crate) fn node_digest_sha256(&self) -> &str {
        self.lock.node_digest_sha256()
    }

    /// Rechecks and builds the fixed private-prefix installer for one operation.
    ///
    /// # Errors
    /// Returns an error instead of handing a future host a changed runtime.
    pub(crate) fn rechecked_npm_prefix(&self) -> Result<NpmGlobalPrefix, AppNodeRuntimeLockError> {
        self.recheck()?;
        Ok(NpmGlobalPrefix::new(
            self.lock.clone(),
            self.private_prefix.clone(),
        ))
    }

    /// Rechecks and binds the app Node runtime to one bounded Ask/Plan launcher.
    ///
    /// # Errors
    /// Returns instead of constructing a launcher when the app-supplied runtime changed.
    pub(crate) fn rechecked_read_only_launcher(
        &self,
        output_limit: usize,
    ) -> Result<NodeReadOnlyHostLauncher, AppNodeRuntimeLockError> {
        self.recheck()?;
        Ok(NodeReadOnlyHostLauncher::new(
            self.lock.clone(),
            output_limit,
        ))
    }
}

fn packaged_node(data_dir: &Path) -> PathBuf {
    let release = std::env::current_exe()
        .ok()
        .map(|executable| packaged_node_from_executable(&executable));
    release.filter(|node| node.is_file()).unwrap_or_else(|| {
        data_dir
            .join("runtime")
            .join("node")
            .join("bin")
            .join(node_name())
    })
}

fn packaged_node_from_executable(executable: &Path) -> PathBuf {
    executable
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("runtime/node/bin")
        .join(node_name())
}

#[cfg(windows)]
const fn node_name() -> &'static str {
    "node.exe"
}

#[cfg(not(windows))]
const fn node_name() -> &'static str {
    "node"
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{AppNodeRuntimeLock, AppNodeRuntimeLockError};

    #[test]
    fn app_runtime_binds_policy_digest_and_private_prefix() {
        let root = tempfile::tempdir().unwrap();
        let node = write_pair(root.path());
        let runtime =
            AppNodeRuntimeLock::capture(Some(node.into_os_string()), &root.path().join("gentd"))
                .unwrap();
        assert_eq!(runtime.node_digest_sha256().len(), 64);
        let install = runtime
            .rechecked_npm_prefix()
            .unwrap()
            .install_archive(std::path::Path::new("/private/verified.tgz"));
        assert!(install.executable.ends_with("bin/node"));
        assert!(install.arguments[0].ends_with("npm-cli.js"));
        assert_eq!(install.arguments[4], "--prefix");
        assert!(install.arguments[5].ends_with("gentd/providers/npm-global"));
        runtime.recheck().unwrap();
    }

    #[test]
    fn changed_app_runtime_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let node = write_pair(root.path());
        let runtime =
            AppNodeRuntimeLock::capture(Some(node.clone().into_os_string()), root.path()).unwrap();
        fs::write(node, "replacement").unwrap();
        assert!(matches!(
            runtime.rechecked_npm_prefix(),
            Err(AppNodeRuntimeLockError::Runtime(_))
        ));
    }

    #[test]
    fn changed_app_runtime_cannot_create_a_read_only_provider_launcher() {
        let root = tempfile::tempdir().unwrap();
        let node = write_pair(root.path());
        let runtime =
            AppNodeRuntimeLock::capture(Some(node.clone().into_os_string()), root.path()).unwrap();
        fs::write(node, "replacement").unwrap();
        assert!(matches!(
            runtime.rechecked_read_only_launcher(1024),
            Err(AppNodeRuntimeLockError::Runtime(_))
        ));
    }

    #[test]
    fn standalone_runtime_prefers_the_explicit_binary() {
        let root = tempfile::tempdir().unwrap();
        let explicit = write_pair(&root.path().join("explicit"));
        let packaged = write_pair(&root.path().join("gentd").join("runtime").join("node"));
        let runtime = AppNodeRuntimeLock::capture_standalone(
            Some(explicit.clone().into_os_string()),
            &root.path().join("gentd"),
        )
        .unwrap();
        fs::write(explicit, "replacement").unwrap();
        assert!(matches!(
            runtime.recheck(),
            Err(AppNodeRuntimeLockError::Runtime(_))
        ));
        assert!(packaged.exists());
    }

    #[test]
    fn standalone_runtime_uses_the_packaged_binary_without_path_lookup() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("gentd");
        let node = write_pair(&data_dir.join("runtime").join("node"));
        let runtime = AppNodeRuntimeLock::capture_standalone(None, &data_dir).unwrap();
        fs::write(node, "replacement").unwrap();
        assert!(matches!(
            runtime.recheck(),
            Err(AppNodeRuntimeLockError::Runtime(_))
        ));
    }

    #[test]
    fn standalone_runtime_prefers_the_installed_release_runtime() {
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("release");
        let node = write_pair(&release.join("runtime").join("node"));
        let resolved = super::packaged_node_from_executable(&release.join("gentd"));
        assert_eq!(resolved, node);
    }

    #[test]
    fn standalone_runtime_requires_an_explicit_or_packaged_binary() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            AppNodeRuntimeLock::capture_standalone(None, root.path()),
            Err(AppNodeRuntimeLockError::Runtime(_))
        ));
    }

    fn write_pair(root: &std::path::Path) -> std::path::PathBuf {
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let node = bin.join(super::node_name());
        fs::write(&node, "node").unwrap();
        fs::write(bin.join(npm_name()), "npm").unwrap();
        let cli = root.join("lib/node_modules/npm/bin");
        fs::create_dir_all(&cli).unwrap();
        fs::write(cli.join("npm-cli.js"), "npm cli").unwrap();
        node
    }

    fn package() -> gent_ports::ApprovedPackageInstall {
        gent_ports::ApprovedPackageInstall {
            provider: "codex".into(),
            package_name: "package".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "a".repeat(64),
        }
    }

    #[cfg(windows)]
    const fn npm_name() -> &'static str {
        "npm.cmd"
    }
    #[cfg(not(windows))]
    const fn npm_name() -> &'static str {
        "npm"
    }
}
