//! Immutable app-supplied Node/npm runtime identity checks.
//!
//! This module only captures and rechecks files. It never installs packages or
//! spawns a process; `gentd` decides whether an approved authority may use it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

/// Canonical identity of the Node runtime and its required npm command files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeRuntimeLock {
    node: LockedRuntimeFile,
    npm: LockedRuntimeFile,
    npm_cli: LockedRuntimeFile,
}

/// One immutable runtime executable identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockedRuntimeFile {
    canonical_path: PathBuf,
    file_identity: String,
    digest_sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum NodeRuntimeLockError {
    #[error("app-supplied Node runtime changed before provisioning")]
    RuntimeChanged,
    #[error("app-supplied npm is not Node's sibling executable")]
    NpmNotSibling,
    #[error("cannot inspect app-supplied Node runtime: {0}")]
    Io(#[from] std::io::Error),
}

impl NodeRuntimeLock {
    /// Captures the canonical Node binary, sibling `npm`, and its CLI module.
    ///
    /// # Errors
    /// Returns an error when either executable cannot be inspected.
    pub fn capture(node: &Path) -> Result<Self, NodeRuntimeLockError> {
        let node = LockedRuntimeFile::capture(node)?;
        let npm = LockedRuntimeFile::capture(
            &node
                .canonical_path
                .parent()
                .ok_or(NodeRuntimeLockError::NpmNotSibling)?
                .join(npm_name()),
        )?;
        let npm_cli = LockedRuntimeFile::capture(&npm_cli_path(&node.canonical_path)?)?;
        Ok(Self { node, npm, npm_cli })
    }

    /// Rechecks the locked files and their Node-sibling relationship.
    ///
    /// # Errors
    /// Returns [`NodeRuntimeLockError::RuntimeChanged`] instead of accepting a
    /// substituted Node/npm runtime.
    pub fn recheck(&self) -> Result<(), NodeRuntimeLockError> {
        self.node.recheck()?;
        self.npm.recheck()?;
        self.npm_cli.recheck()?;
        let expected = LockedRuntimeFile::capture(
            &self
                .node
                .canonical_path
                .parent()
                .ok_or(NodeRuntimeLockError::NpmNotSibling)?
                .join(npm_name()),
        )?;
        (expected == self.npm)
            .then_some(())
            .ok_or(NodeRuntimeLockError::NpmNotSibling)?;
        let expected_cli = LockedRuntimeFile::capture(&npm_cli_path(&self.node.canonical_path)?)?;
        (expected_cli == self.npm_cli)
            .then_some(())
            .ok_or(NodeRuntimeLockError::NpmNotSibling)
    }

    /// SHA-256 identity used to bind a signed package policy to this Node binary.
    #[must_use]
    pub fn node_digest_sha256(&self) -> &str {
        &self.node.digest_sha256
    }

    /// Canonical npm shim path, retained only to prove its sibling relationship.
    #[must_use]
    pub fn npm_path(&self) -> &Path {
        &self.npm.canonical_path
    }

    /// Canonical npm CLI module executed through the locked Node binary.
    #[must_use]
    pub fn npm_cli_path(&self) -> &Path {
        &self.npm_cli.canonical_path
    }

    /// Canonical Node executable identity for audit and diagnostics.
    #[must_use]
    pub fn node_path(&self) -> &Path {
        &self.node.canonical_path
    }
}

fn npm_cli_path(node: &Path) -> Result<PathBuf, NodeRuntimeLockError> {
    let bin = node.parent().ok_or(NodeRuntimeLockError::NpmNotSibling)?;
    #[cfg(windows)]
    let root = bin;
    #[cfg(not(windows))]
    let root = bin.parent().ok_or(NodeRuntimeLockError::NpmNotSibling)?;
    Ok(root.join("lib/node_modules/npm/bin/npm-cli.js"))
}

impl LockedRuntimeFile {
    fn capture(path: &Path) -> Result<Self, NodeRuntimeLockError> {
        let canonical_path = fs::canonicalize(path)?;
        let metadata = fs::metadata(&canonical_path)?;
        let bytes = fs::read(&canonical_path)?;
        Ok(Self {
            canonical_path,
            file_identity: format!(
                "{}:{}",
                metadata.len(),
                metadata
                    .modified()?
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_nanos())
            ),
            digest_sha256: hex::encode(Sha256::digest(bytes)),
        })
    }

    fn recheck(&self) -> Result<(), NodeRuntimeLockError> {
        (Self::capture(&self.canonical_path)? == *self)
            .then_some(())
            .ok_or(NodeRuntimeLockError::RuntimeChanged)
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{NodeRuntimeLock, NodeRuntimeLockError};

    #[test]
    fn lock_captures_canonical_pair_and_node_digest() {
        let root = tempfile::tempdir().unwrap();
        let node = write_pair(root.path());
        let lock = NodeRuntimeLock::capture(&node).unwrap();
        assert_eq!(lock.node_path(), node.canonicalize().unwrap());
        assert!(lock.npm_path().ends_with(super::npm_name()));
        assert!(lock.npm_cli_path().ends_with("npm-cli.js"));
        assert_eq!(lock.node_digest_sha256().len(), 64);
        lock.recheck().unwrap();
    }

    #[test]
    fn modified_npm_is_refused_before_provisioning() {
        let root = tempfile::tempdir().unwrap();
        let node = write_pair(root.path());
        let lock = NodeRuntimeLock::capture(&node).unwrap();
        fs::write(root.path().join("bin").join(super::npm_name()), "new npm").unwrap();
        assert!(matches!(
            lock.recheck(),
            Err(NodeRuntimeLockError::RuntimeChanged)
        ));
    }

    #[test]
    fn modified_npm_cli_is_refused_before_provisioning() {
        let root = tempfile::tempdir().unwrap();
        let node = write_pair(root.path());
        let lock = NodeRuntimeLock::capture(&node).unwrap();
        fs::write(
            root.path().join("lib/node_modules/npm/bin/npm-cli.js"),
            "new npm cli",
        )
        .unwrap();
        assert!(matches!(
            lock.recheck(),
            Err(NodeRuntimeLockError::RuntimeChanged)
        ));
    }

    #[test]
    fn modified_node_is_refused_before_provisioning() {
        let root = tempfile::tempdir().unwrap();
        let node = write_pair(root.path());
        let lock = NodeRuntimeLock::capture(&node).unwrap();
        fs::write(&node, "new node").unwrap();
        assert!(matches!(
            lock.recheck(),
            Err(NodeRuntimeLockError::RuntimeChanged)
        ));
    }

    fn write_pair(root: &std::path::Path) -> std::path::PathBuf {
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        let node = bin.join("node");
        fs::write(&node, "node").unwrap();
        fs::write(bin.join(super::npm_name()), "npm").unwrap();
        let cli = root.join("lib/node_modules/npm/bin");
        fs::create_dir_all(&cli).unwrap();
        fs::write(cli.join("npm-cli.js"), "npm cli").unwrap();
        node
    }
}
