//! Explicit vendor-installer execution with fixed argument vectors and no shell.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use gent_ports::ApprovedPackageInstall;

use crate::node_runtime_lock::{NodeRuntimeLock, NodeRuntimeLockError};

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
                self.runtime.npm_cli_path().to_string_lossy().into_owned(),
                "pack".into(),
                "--ignore-scripts".into(),
                "--json".into(),
                "--pack-destination".into(),
                staging.to_string_lossy().into_owned(),
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
                self.runtime.npm_cli_path().to_string_lossy().into_owned(),
                "install".into(),
                "--ignore-scripts".into(),
                "--global".into(),
                "--prefix".into(),
                self.prefix.to_string_lossy().into_owned(),
                archive.to_string_lossy().into_owned(),
            ],
        }
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
        let node_bin =
            self.runtime.node_path().parent().ok_or_else(|| {
                InstallerError::Runtime("locked Node has no bin directory".into())
            })?;
        crate::process::configure_locked_node_environment(command, node_bin)
            .map_err(|error| InstallerError::Launch(error.to_string()))?;
        let cache = self.prefix.join(".npm-cache");
        let config = self.prefix.join(".npmrc");
        command
            .env("npm_config_prefix", &self.prefix)
            .env("NPM_CONFIG_PREFIX", &self.prefix)
            .env("npm_config_cache", cache)
            .env("NPM_CONFIG_CACHE", self.prefix.join(".npm-cache"))
            .env("npm_config_userconfig", config)
            .env("NPM_CONFIG_USERCONFIG", self.prefix.join(".npmrc"));
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
    #[error("signed package integrity is not a SHA-512 SRI digest")]
    InvalidIntegrity,
    #[error("packed tarball does not match signed package integrity")]
    IntegrityMismatch,
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    use gent_ports::ApprovedPackageInstall;

    #[test]
    fn npm_commands_use_only_the_private_prefix_and_fixed_arguments() {
        let root = tempfile::tempdir().unwrap();
        let npm = super::NpmGlobalPrefix::new(
            runtime(root.path()),
            PathBuf::from("/private/gentd/providers/npm-global"),
        );
        let package = ApprovedPackageInstall {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "0.147.0".into(),
            integrity: "sha512-test".into(),
            package_policy_digest_sha256: "a".repeat(64),
        };
        let packed = npm.pack(&package, std::path::Path::new("/private/staging"));
        assert!(packed.executable.ends_with("bin/node"));
        assert_eq!(packed.arguments[0], npm_cli(root.path()).to_string_lossy());
        assert_eq!(packed.arguments[1], "pack");
        assert_eq!(packed.arguments[2], "--ignore-scripts");
        assert_eq!(packed.arguments[3], "--json");
        assert_eq!(packed.arguments[4], "--pack-destination");
        assert_eq!(packed.arguments[5], "/private/staging");
        assert_eq!(packed.arguments[6], "@openai/codex@0.147.0");
        let installed = npm.install_archive(std::path::Path::new("/private/staging/codex.tgz"));
        assert_eq!(
            installed.arguments[5],
            "/private/gentd/providers/npm-global"
        );
        assert_eq!(installed.arguments[6], "/private/staging/codex.tgz");
        assert_eq!(installed.arguments[2], "--ignore-scripts");
    }

    #[test]
    fn private_npm_command_clears_host_node_and_npm_controls() {
        let root = tempfile::tempdir().unwrap();
        let prefix = root.path().join("private-prefix");
        let npm = super::NpmGlobalPrefix::new(runtime(root.path()), prefix.clone());
        let mut command = Command::new("ignored");
        for name in ["NODE_OPTIONS", "NODE_PATH", "NPM_CONFIG_REGISTRY"] {
            command.env(name, "host-value");
        }
        npm.configure_command(&mut command).unwrap();
        let values = command
            .get_envs()
            .map(|(name, value)| (name.to_string_lossy().into_owned(), value.is_some()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(values.get("NODE_OPTIONS"), Some(&false));
        assert_eq!(values.get("NODE_PATH"), Some(&false));
        assert_eq!(values.get("NPM_CONFIG_REGISTRY"), Some(&false));
        assert_eq!(
            values.get("npm_config_prefix"),
            Some(&true),
            "npm receives only the private prefix"
        );
        assert_eq!(values.get("npm_config_userconfig"), Some(&true));
    }

    #[cfg(unix)]
    #[test]
    fn private_npm_path_contains_only_locked_node_and_system_interpreters() {
        let root = tempfile::tempdir().unwrap();
        let npm = super::NpmGlobalPrefix::new(runtime(root.path()), root.path().join("prefix"));
        let mut command = Command::new("ignored");
        npm.configure_command(&mut command).unwrap();
        let path = command
            .get_envs()
            .find_map(|(name, value)| (name == "PATH").then_some(value))
            .flatten()
            .unwrap();
        assert_eq!(
            std::env::split_paths(path).collect::<Vec<_>>(),
            vec![
                root.path().join("bin").canonicalize().unwrap(),
                "/usr/bin".into(),
                "/bin".into()
            ]
        );
    }

    fn runtime(root: &Path) -> crate::node_runtime_lock::NodeRuntimeLock {
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("node"), "node").unwrap();
        fs::write(bin.join(npm_name()), "npm").unwrap();
        let cli = root.join("lib/node_modules/npm/bin");
        fs::create_dir_all(&cli).unwrap();
        fs::write(cli.join("npm-cli.js"), "npm cli").unwrap();
        crate::node_runtime_lock::NodeRuntimeLock::capture(&bin.join("node")).unwrap()
    }

    fn npm_cli(root: &Path) -> PathBuf {
        root.join("lib/node_modules/npm/bin/npm-cli.js")
            .canonicalize()
            .unwrap()
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
