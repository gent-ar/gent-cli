#![allow(dead_code)]

//! Daemon-owned public executable resolution for a future authority-gated composition.
//!
//! Observer composition intentionally leaves this component dormant; constructing it there would
//! make a provider version probe reachable before the evidence gate allows authority transfer.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use gent_drivers::discovery::{
    DiscoveryError, ExecutableDiscovery, ProbeError, PublicProvider, VersionProbe, inspect,
};
use gent_drivers::lock::capture;
use gent_ports::{PublicProviderResolver, PublicProviderRunError};
use gent_types::RunVersionLock;

use crate::compatibility_assessment::CompatibilityAssessment;

/// Resolves only daemon-discovered public executables and binds their signed lock entry.
#[derive(Debug)]
pub struct DaemonProviderResolver<D, P> {
    compatibility: CompatibilityAssessment,
    discovery: D,
    probe: P,
}

impl<D, P> DaemonProviderResolver<D, P> {
    /// Creates a resolver for a future authority-gated daemon composition.
    #[must_use]
    pub(crate) fn new(compatibility: CompatibilityAssessment, discovery: D, probe: P) -> Self {
        Self {
            compatibility,
            discovery,
            probe,
        }
    }
}

impl<D: ExecutableDiscovery, P: VersionProbe> PublicProviderResolver
    for DaemonProviderResolver<D, P>
{
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        let public = public_provider(provider)?;
        let installation = inspect(public, &self.discovery, &self.probe)
            .map_err(|_| PublicProviderRunError::CompatibilityDenied)?
            .ok_or(PublicProviderRunError::CompatibilityDenied)?;
        let observed = capture(
            provider,
            &installation.executable.path,
            &installation.version,
            "unbound",
        )
        .map_err(|_| PublicProviderRunError::CompatibilityDenied)?;
        self.compatibility.bind_observed_lock(observed)
    }
}

fn public_provider(provider: &str) -> Result<PublicProvider, PublicProviderRunError> {
    match provider {
        "claude" => Ok(PublicProvider::Claude),
        "codex" => Ok(PublicProvider::Codex),
        _ => Err(PublicProviderRunError::CompatibilityDenied),
    }
}

/// Private-prefix-first discovery for a future authority-mode composition.
///
/// The native app supplies Node to Gent, and Gent installs public provider CLIs only below its
/// own data directory. This adapter ensures a later approved host resolves that installation
/// before it considers a user-controlled `PATH` fallback.
#[derive(Clone, Debug)]
pub struct PrivatePrefixFirstDiscovery<D> {
    prefix: PathBuf,
    fallback: D,
}

impl<D> PrivatePrefixFirstDiscovery<D> {
    /// Binds one Gent-owned npm prefix and a separately injected fallback discovery port.
    #[must_use]
    pub(crate) fn new(prefix: PathBuf, fallback: D) -> Self {
        Self { prefix, fallback }
    }
}

impl<D: ExecutableDiscovery> ExecutableDiscovery for PrivatePrefixFirstDiscovery<D> {
    fn find(&self, name: &str) -> Result<Option<PathBuf>, DiscoveryError> {
        let candidate = self.prefix.join("bin").join(provider_binary_name(name));
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
        self.fallback.find(name)
    }
}

#[cfg(windows)]
fn provider_binary_name(name: &str) -> String {
    format!("{name}.cmd")
}

#[cfg(not(windows))]
fn provider_binary_name(name: &str) -> String {
    name.into()
}

/// PATH-based fallback adapter for a future authority-mode composition.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemExecutableDiscovery;

impl ExecutableDiscovery for SystemExecutableDiscovery {
    fn find(&self, name: &str) -> Result<Option<PathBuf>, DiscoveryError> {
        let Some(paths) = env::var_os("PATH") else {
            return Ok(None);
        };
        Ok(env::split_paths(&paths)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file()))
    }
}

/// Fixed-argument public version-probe adapter for a future authority-mode composition.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemVersionProbe;

impl VersionProbe for SystemVersionProbe {
    fn probe(&self, executable: &Path, argument: &str) -> Result<String, ProbeError> {
        let output = Command::new(executable)
            .arg(argument)
            .output()
            .map_err(|error| ProbeError::Failed(error.to_string()))?;
        if !output.status.success() {
            return Err(ProbeError::Failed(format!("exit status {}", output.status)));
        }
        let version = String::from_utf8(output.stdout)
            .map_err(|_| ProbeError::Failed("version output is not UTF-8".into()))?;
        let version = version.trim();
        if version.is_empty() || version.len() > 1024 {
            return Err(ProbeError::Failed("version output is invalid".into()));
        }
        Ok(version.into())
    }
}
