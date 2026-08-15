//! Explicit vendor-installer execution with fixed argument vectors and no shell.

use std::process::Command;

/// A reviewed installer command. Arguments are never interpreted by a shell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallerInvocation {
    pub executable: String,
    pub arguments: Vec<String>,
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
