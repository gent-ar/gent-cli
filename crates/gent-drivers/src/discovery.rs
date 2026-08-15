//! Public executable discovery and version-probe ports, with no process implementation.

use std::path::{Path, PathBuf};

/// Public providers that Gent can inspect when explicitly requested.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicProvider {
    Claude,
    Codex,
}

impl PublicProvider {
    /// The executable name used by this provider's public CLI.
    #[must_use]
    pub const fn executable_name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// The safe public argument used to ask for the installed version.
    #[must_use]
    pub const fn version_argument(self) -> &'static str {
        "--version"
    }
}

/// A discovered executable, retained without starting it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredExecutable {
    pub provider: PublicProvider,
    pub path: PathBuf,
}

/// Resolves an executable from a caller-controlled search environment.
pub trait ExecutableDiscovery: Send + Sync {
    /// Returns no value when the provider is not installed.
    ///
    /// # Errors
    /// Returns an error when the configured search environment cannot be inspected.
    fn find(&self, executable_name: &str) -> Result<Option<PathBuf>, DiscoveryError>;
}

/// Obtains a public version string. Implementations may execute only a version probe.
pub trait VersionProbe: Send + Sync {
    /// Invokes the provider's documented version command, never a session command.
    ///
    /// # Errors
    /// Returns an error when the version command cannot complete or be parsed.
    fn probe(&self, executable: &Path, version_argument: &str) -> Result<String, ProbeError>;
}

/// Result of discovering and identifying a provider without starting a provider session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderInstallation {
    pub executable: DiscoveredExecutable,
    pub version: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("executable discovery failed: {0}")]
    Failed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("version probe failed: {0}")]
    Failed(String),
}

/// Combines independent discovery and probe ports into an installation report.
///
/// # Errors
/// Returns discovery or probe errors without attempting to launch a provider session.
pub fn inspect(
    provider: PublicProvider,
    discovery: &dyn ExecutableDiscovery,
    probe: &dyn VersionProbe,
) -> Result<Option<ProviderInstallation>, InspectError> {
    let name = provider.executable_name();
    let Some(path) = discovery.find(name)? else {
        return Ok(None);
    };
    let version = probe.probe(&path, provider.version_argument())?;
    Ok(Some(ProviderInstallation {
        executable: DiscoveredExecutable { provider, path },
        version,
    }))
}

#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    #[error(transparent)]
    Probe(#[from] ProbeError),
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{ExecutableDiscovery, PublicProvider, VersionProbe, inspect};

    struct Found;

    impl ExecutableDiscovery for Found {
        fn find(&self, _: &str) -> Result<Option<PathBuf>, super::DiscoveryError> {
            Ok(Some(PathBuf::from("/tools/codex")))
        }
    }

    struct Version;

    impl VersionProbe for Version {
        fn probe(&self, _: &Path, argument: &str) -> Result<String, super::ProbeError> {
            assert_eq!(argument, "--version");
            Ok("1.2.3".into())
        }
    }

    #[test]
    fn inspect_combines_only_discovery_and_version_probe() {
        let installation = inspect(PublicProvider::Codex, &Found, &Version)
            .unwrap()
            .unwrap();
        assert_eq!(installation.executable.path, PathBuf::from("/tools/codex"));
        assert_eq!(installation.version, "1.2.3");
    }

    #[test]
    fn public_provider_has_explicit_commands() {
        assert_eq!(PublicProvider::Claude.executable_name(), "claude");
        assert_eq!(PublicProvider::Codex.version_argument(), "--version");
    }
}
