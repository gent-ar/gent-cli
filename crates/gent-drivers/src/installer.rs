//! Explicit vendor-installer execution with fixed argument vectors and no shell.

use std::path::{Path, PathBuf};

use gent_ports::ApprovedPackageInstall;

/// A reviewed installer command. Arguments are never interpreted by a shell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallerInvocation {
    pub executable: String,
    pub arguments: Vec<String>,
}

/// Private `npm` prefix used only for daemon-owned public provider installation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NpmGlobalPrefix {
    npm: PathBuf,
    prefix: PathBuf,
}

impl NpmGlobalPrefix {
    #[must_use]
    pub fn new(npm: PathBuf, prefix: PathBuf) -> Self {
        Self { npm, prefix }
    }

    /// Builds a fixed package fetch into an already-created private staging directory.
    #[must_use]
    pub fn pack(&self, package: &ApprovedPackageInstall, staging: &Path) -> InstallerInvocation {
        InstallerInvocation {
            executable: self.npm.to_string_lossy().into_owned(),
            arguments: vec![
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
            executable: self.npm.to_string_lossy().into_owned(),
            arguments: vec![
                "install".into(),
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
    use std::path::PathBuf;

    use gent_ports::ApprovedPackageInstall;

    #[test]
    fn npm_commands_use_only_the_private_prefix_and_fixed_arguments() {
        let npm = super::NpmGlobalPrefix::new(
            PathBuf::from("/app/node/bin/npm"),
            PathBuf::from("/private/gentd/providers/npm-global"),
        );
        let package = ApprovedPackageInstall {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "0.147.0".into(),
            integrity: "sha512-test".into(),
        };
        let packed = npm.pack(&package, std::path::Path::new("/private/staging"));
        assert_eq!(packed.executable, "/app/node/bin/npm");
        assert_eq!(
            packed.arguments,
            [
                "pack",
                "--ignore-scripts",
                "--json",
                "--pack-destination",
                "/private/staging",
                "@openai/codex@0.147.0"
            ]
        );
        let installed = npm.install_archive(std::path::Path::new("/private/staging/codex.tgz"));
        assert_eq!(
            installed.arguments[3],
            "/private/gentd/providers/npm-global"
        );
        assert_eq!(installed.arguments[4], "/private/staging/codex.tgz");
    }
}
