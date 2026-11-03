//! Explicit vendor-installer execution with fixed argument vectors and no shell.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use gent_ports::ApprovedPackageInstall;

use crate::node_runtime_lock::{NodeRuntimeLock, NodeRuntimeLockError};

#[path = "installer_npm_paths.rs"]
mod npm_paths;
pub(super) use npm_paths::{
    archive_argument as npm_archive_argument, filesystem_path as npm_archive_path,
};
use npm_paths::{checked_filesystem_path, filesystem_path};

/// A reviewed installer command. Arguments are never interpreted by a shell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallerInvocation {
    pub executable: String,
    pub arguments: Vec<String>,
}

/// Private `npm` prefix used only for daemon-owned public provider installation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NpmGlobalPrefix {
    runtime: NodeRuntimeLock,
    prefix: PathBuf,
}

impl NpmGlobalPrefix {
    #[must_use]
    pub fn new(runtime: NodeRuntimeLock, prefix: PathBuf) -> Self {
        Self { runtime, prefix }
    }

    /// Builds a fixed package fetch into an already-created private staging directory.
    #[must_use]
    pub fn pack(&self, package: &ApprovedPackageInstall, staging: &Path) -> InstallerInvocation {
        InstallerInvocation {
            executable: self.runtime.node_path().to_string_lossy().into_owned(),
            arguments: vec![
                filesystem_path(self.runtime.npm_cli_path()),
                "pack".into(),
                "--ignore-scripts".into(),
                "--json".into(),
                "--pack-destination".into(),
                filesystem_path(staging),
                package.selector(),
            ],
        }
    }

    /// Builds an installation from an already-verified tarball only.
    #[must_use]
    pub fn install_archive(&self, archive: &Path) -> InstallerInvocation {
        InstallerInvocation {
            executable: self.runtime.node_path().to_string_lossy().into_owned(),
            arguments: vec![
                filesystem_path(self.runtime.npm_cli_path()),
                "install".into(),
                "--ignore-scripts".into(),
                "--global".into(),
                "--prefix".into(),
                filesystem_path(&self.prefix),
                "--offline".into(),
                npm_archive_argument(archive),
            ],
        }
    }

    #[must_use]
    pub fn install_package(&self, package: &str) -> InstallerInvocation {
        InstallerInvocation {
            executable: self.runtime.node_path().to_string_lossy().into_owned(),
            arguments: vec![
                filesystem_path(self.runtime.npm_cli_path()),
                "install".into(),
                "--global".into(),
                "--prefix".into(),
                filesystem_path(&self.prefix),
                "--no-audit".into(),
                "--no-fund".into(),
                package.into(),
            ],
        }
    }

    /// Rejects unsupported npm global roots before staging or executing a package operation.
    pub(super) fn validate_prefix(&self) -> Result<(), InstallerError> {
        npm_paths::require_supported_prefix(&self.prefix)?;
        checked_filesystem_path(&self.prefix).map(|_| ())
    }

    pub(super) fn validate_filesystem_path(path: &Path) -> Result<(), InstallerError> {
        checked_filesystem_path(path).map(|_| ())
    }

    /// Returns the daemon-owned package prefix, used to make private staging directories.
    #[must_use]
    pub fn prefix(&self) -> &Path {
        &self.prefix
    }

    /// Rechecks the entire locked Node/npm command chain before an npm process starts.
    ///
    /// # Errors
    /// Returns when Node, npm, or the npm CLI module changed after capture.
    pub fn recheck(&self) -> Result<(), NodeRuntimeLockError> {
        self.runtime.recheck()
    }

    /// Applies the locked runtime and private npm configuration to one fixed npm command.
    ///
    /// # Errors
    /// Returns when the captured Node binary cannot supply a safe command environment.
    pub fn configure_command(&self, command: &mut Command) -> Result<(), InstallerError> {
        self.validate_prefix()?;
        let prefix = checked_filesystem_path(&self.prefix)?;
        checked_filesystem_path(self.runtime.npm_cli_path())?;
        let node_bin =
            self.runtime.node_path().parent().ok_or_else(|| {
                InstallerError::Runtime("locked Node has no bin directory".into())
            })?;
        let node_bin = checked_filesystem_path(node_bin)?;
        crate::process::configure_locked_node_environment(command, Path::new(&node_bin))
            .map_err(|error| InstallerError::Launch(error.to_string()))?;
        // Remove every inherited npm configuration control before setting private paths.
        let controls = std::env::vars_os()
            .map(|(key, _)| key)
            .chain(command.get_envs().map(|(key, _)| key.to_owned()))
            .filter(|key| {
                key.to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("NPM_CONFIG_")
            })
            .collect::<Vec<_>>();
        for key in controls {
            command.env_remove(key);
        }
        let cache = checked_filesystem_path(&self.prefix.join(".npm-cache"))?;
        let config = checked_filesystem_path(&self.prefix.join(".npmrc"))?;
        let global = checked_filesystem_path(&self.prefix.join("etc").join("npmrc"))?;
        command
            .env("npm_config_prefix", &prefix)
            .env("NPM_CONFIG_PREFIX", &prefix)
            .env("npm_config_cache", &cache)
            .env("NPM_CONFIG_CACHE", &cache)
            .env("npm_config_userconfig", &config)
            .env("NPM_CONFIG_USERCONFIG", &config)
            .env("npm_config_globalconfig", &global)
            .env("NPM_CONFIG_GLOBALCONFIG", &global);
        Ok(())
    }
}

/// Runs a signed, exact provider install to completion.
pub trait DependencyInstaller: Clone + Send + Sync {
    /// Packs, verifies, and installs one policy-approved package.
    ///
    /// # Errors
    /// Returns an error when the installer cannot start or exits unsuccessfully.
    fn install(
        &self,
        npm: &NpmGlobalPrefix,
        package: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError>;
}

/// The operating-system implementation used only after explicit client consent.
#[derive(Clone, Debug, Default)]
pub struct SystemDependencyInstaller;

impl DependencyInstaller for SystemDependencyInstaller {
    fn install(
        &self,
        npm: &NpmGlobalPrefix,
        package: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        crate::npm_pack_install::VerifiedNpmInstaller.install(npm, package)
    }
}

/// Failure to launch or complete an explicit vendor installer.
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum InstallerError {
    #[error("installer could not start: {0}")]
    Launch(String),
    #[error("installer exited unsuccessfully: {0}")]
    Failed(String),
    #[error("installer filesystem operation failed: {0}")]
    Io(String),
    #[error("locked Node/npm runtime changed: {0}")]
    Runtime(String),
    #[error("npm pack did not return one valid JSON artifact")]
    PackOutput,
    #[error("npm pack returned an unsafe artifact path")]
    InvalidArtifact,
    /// Returned before any package staging or npm process, never after an install effect.
    #[error(
        "npm global installation requires a local Windows drive; UNC prefixes are unsupported by the bundled npm filesystem resolver"
    )]
    UnsupportedPrefix,
    #[error("signed package integrity is not a SHA-512 SRI digest")]
    InvalidIntegrity,
    #[error("packed tarball does not match signed package integrity")]
    IntegrityMismatch,
}

#[cfg(test)]
#[path = "installer_tests.rs"]
mod tests;
