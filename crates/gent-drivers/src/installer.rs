//! Explicit vendor-installer execution with fixed argument vectors and no shell.

use std::{path::PathBuf, process::Command};

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

    /// Builds a fixed global package installation without a shell or ambient `PATH` lookup.
    #[must_use]
    pub fn install(&self, package: &ApprovedPackageInstall) -> InstallerInvocation {
        InstallerInvocation {
            executable: self.npm.to_string_lossy().into_owned(),
            arguments: vec![
                "install".into(),
                "--global".into(),
                "--prefix".into(),
                self.prefix.to_string_lossy().into_owned(),
                package.selector(),
            ],
        }
    }
}

/// Runs an already-approved provider installer to completion.
pub trait DependencyInstaller: Clone + Send + Sync {
    /// Executes one explicit installer request and waits for its terminal exit.
    ///
    /// # Errors
    /// Returns an error when the installer cannot start or exits unsuccessfully.
    fn execute(&self, invocation: &InstallerInvocation) -> Result<(), InstallerError>;
}

/// The operating-system implementation used only after explicit client consent.
#[derive(Clone, Debug, Default)]
pub struct SystemDependencyInstaller;

impl DependencyInstaller for SystemDependencyInstaller {
    fn execute(&self, invocation: &InstallerInvocation) -> Result<(), InstallerError> {
        let status = Command::new(&invocation.executable)
            .args(&invocation.arguments)
            .status()
            .map_err(|error| InstallerError::Launch(error.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(InstallerError::Failed(status.to_string()))
        }
    }
}

/// Failure to launch or complete an explicit vendor installer.
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum InstallerError {
    #[error("installer could not start: {0}")]
    Launch(String),
    #[error("installer exited unsuccessfully: {0}")]
    Failed(String),
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gent_ports::ApprovedPackageInstall;

    use super::{
        DependencyInstaller, InstallerError, InstallerInvocation, SystemDependencyInstaller,
    };

    #[test]
    fn system_installer_waits_for_a_successful_command() {
        let invocation = success_command();
        assert!(SystemDependencyInstaller.execute(&invocation).is_ok());
    }

    #[test]
    fn system_installer_reports_nonzero_exit_status() {
        let invocation = failure_command();
        assert!(matches!(
            SystemDependencyInstaller.execute(&invocation),
            Err(InstallerError::Failed(_))
        ));
    }

    #[test]
    fn npm_install_uses_only_the_private_prefix_and_fixed_arguments() {
        let invocation = super::NpmGlobalPrefix::new(
            PathBuf::from("/app/node/bin/npm"),
            PathBuf::from("/private/gentd/providers/npm-global"),
        )
        .install(&ApprovedPackageInstall {
            provider: "codex".into(),
            package_name: "@openai/codex".into(),
            version: "0.147.0".into(),
            integrity: "sha512-test".into(),
        });
        assert_eq!(invocation.executable, "/app/node/bin/npm");
        assert_eq!(
            invocation.arguments,
            [
                "install",
                "--global",
                "--prefix",
                "/private/gentd/providers/npm-global",
                "@openai/codex@0.147.0"
            ]
        );
    }

    #[cfg(unix)]
    fn success_command() -> InstallerInvocation {
        InstallerInvocation {
            executable: "/usr/bin/true".into(),
            arguments: vec![],
        }
    }

    #[cfg(unix)]
    fn failure_command() -> InstallerInvocation {
        InstallerInvocation {
            executable: "/usr/bin/false".into(),
            arguments: vec![],
        }
    }

    #[cfg(windows)]
    fn success_command() -> InstallerInvocation {
        InstallerInvocation {
            executable: "cmd".into(),
            arguments: vec!["/C".into(), "exit 0".into()],
        }
    }

    #[cfg(windows)]
    fn failure_command() -> InstallerInvocation {
        InstallerInvocation {
            executable: "cmd".into(),
            arguments: vec!["/C".into(), "exit 1".into()],
        }
    }
}
