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
    #[error("bundled Node runtime is unavailable; set GENT_NODE_BINARY")]
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

    fn capture(node: Option<OsString>, data_dir: &Path) -> Result<Self, AppNodeRuntimeLockError> {
        let node = node.ok_or(AppNodeRuntimeLockError::MissingNode)?;
        Ok(Self {
            lock: NodeRuntimeLock::capture(Path::new(&node))?,
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
            self.lock.npm_path().into(),
            self.private_prefix.clone(),
        ))
    }
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
        assert!(
            runtime
                .rechecked_npm_prefix()
                .unwrap()
                .install(&package())
                .arguments[3]
                .ends_with("gentd/providers/npm-global")
        );
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

    fn write_pair(root: &std::path::Path) -> std::path::PathBuf {
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        let node = bin.join("node");
        fs::write(&node, "node").unwrap();
        fs::write(bin.join(npm_name()), "npm").unwrap();
        node
    }

    fn package() -> gent_ports::ApprovedPackageInstall {
        gent_ports::ApprovedPackageInstall {
            provider: "codex".into(),
            package_name: "package".into(),
            version: "1.0.0".into(),
            integrity: "sha512-test".into(),
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
