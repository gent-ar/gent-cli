#![allow(dead_code)]

//! Daemon-owned public executable resolution for a future authority-gated composition.
//!
//! Observer composition intentionally leaves this component dormant; constructing it there would
//! make a provider version probe reachable before the evidence gate allows authority transfer.

use std::env;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

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

/// Provider-scoped resolver for a future approved Codex composition.
///
/// The wrapper rejects every other public or private provider before delegation, so an approved
/// Codex record cannot cause Claude discovery or probing.
#[derive(Debug)]
pub(crate) struct CodexOnlyResolver<D, P> {
    inner: DaemonProviderResolver<D, P>,
}

impl<D, P> CodexOnlyResolver<D, P> {
    /// Binds an already configured daemon resolver without performing discovery or probing.
    #[must_use]
    pub(crate) const fn new(inner: DaemonProviderResolver<D, P>) -> Self {
        Self { inner }
    }
}

impl<D: ExecutableDiscovery, P: VersionProbe> PublicProviderResolver for CodexOnlyResolver<D, P> {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        if provider != "codex" {
            return Err(PublicProviderRunError::CompatibilityDenied);
        }
        self.inner.resolve(provider)
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

/// Gent-private executable discovery for an approved public-driver composition.
///
/// Unlike diagnostic discovery, authority-mode resolution must never fall back to a
/// user-controlled `PATH`. Provisioning is a separate consented operation that places the
/// locked executable below this prefix before this discovery is reachable.
#[derive(Clone, Debug)]
pub struct PrivatePrefixDiscovery {
    prefix: PathBuf,
}

impl PrivatePrefixDiscovery {
    /// Binds the one Gent-owned npm prefix allowed to supply an authority-mode executable.
    #[must_use]
    pub(crate) fn new(prefix: PathBuf) -> Self {
        Self { prefix }
    }
}

impl ExecutableDiscovery for PrivatePrefixDiscovery {
    fn find(&self, name: &str) -> Result<Option<PathBuf>, DiscoveryError> {
        let candidate = self.prefix.join("bin").join(provider_binary_name(name));
        Ok(candidate.is_file().then_some(candidate))
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
        probe_with_timeout(executable, argument, Duration::from_secs(5))
    }
}

const MAX_VERSION_BYTES: usize = 1024;
const DRAIN_BUFFER_BYTES: usize = 4096;

pub(crate) fn probe_with_timeout(
    executable: &Path,
    argument: &str,
    timeout: Duration,
) -> Result<String, ProbeError> {
    if argument != "--version" {
        return Err(ProbeError::Failed(
            "version probe argument is invalid".into(),
        ));
    }
    let mut child = Command::new(executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| ProbeError::Failed("version probe could not start".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ProbeError::Failed("version probe stdout is unavailable".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ProbeError::Failed("version probe stderr is unavailable".into()))?;
    let output = bounded_reader(stdout, MAX_VERSION_BYTES + 1);
    discard_reader(stderr);
    let deadline = Instant::now() + timeout;
    let status = wait_for_exit(&mut child, deadline)?;
    if !status.success() {
        return Err(ProbeError::Failed(
            "version probe exited unsuccessfully".into(),
        ));
    }
    let version = wait_for_output(&output, deadline)?;
    let version = String::from_utf8(version)
        .map_err(|_| ProbeError::Failed("version output is not UTF-8".into()))?;
    let version = version.trim();
    if version.is_empty() || version.len() > MAX_VERSION_BYTES {
        return Err(ProbeError::Failed("version output is invalid".into()));
    }
    Ok(version.into())
}

fn bounded_reader<R: Read + Send + 'static>(
    reader: R,
    maximum: usize,
) -> mpsc::Receiver<io::Result<Vec<u8>>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(drain_bounded(reader, maximum));
    });
    receiver
}

fn discard_reader<R: Read + Send + 'static>(reader: R) {
    std::thread::spawn(move || {
        let _ = drain_bounded(reader, 0);
    });
}

fn drain_bounded<R: Read>(mut reader: R, maximum: usize) -> io::Result<Vec<u8>> {
    let mut kept = Vec::with_capacity(maximum);
    let mut buffer = [0; DRAIN_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(kept);
        }
        let remaining = maximum.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..read.min(remaining)]);
    }
}

fn wait_for_exit(
    child: &mut Child,
    deadline: Instant,
) -> Result<std::process::ExitStatus, ProbeError> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(_) => {
                return Err(ProbeError::Failed(
                    "version probe could not be polled".into(),
                ));
            }
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProbeError::Failed("version probe timed out".into()));
        };
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

fn wait_for_output(
    output: &mpsc::Receiver<io::Result<Vec<u8>>>,
    deadline: Instant,
) -> Result<Vec<u8>, ProbeError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| ProbeError::Failed("version probe timed out".into()))?;
    output
        .recv_timeout(remaining)
        .map_err(|_| ProbeError::Failed("version probe output did not close".into()))?
        .map_err(|_| ProbeError::Failed("version probe output could not be read".into()))
}
