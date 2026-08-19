//! Daemon-only sandbox values, never serializable or client-provided, for contained provider spawn.
use crate::{RunVersionLock, SandboxEnforcement};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
/// Network posture that a sandbox backend must enforce, rather than suggest to a provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SandboxNetworkPolicy {
    Disabled,
    ReviewedEgress { policy_digest_sha256: String },
}

/// Canonical resource limits which a backend must apply to the whole provider process tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxResourceLimits {
    pub max_processes: u16,
    pub max_memory_bytes: u64,
    pub max_cpu_time_ms: u64,
}

/// Canonical, credential-free containment profile prepared by the daemon process edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxLaunchProfile {
    workspace_root: PathBuf,
    readable_roots: Vec<PathBuf>,
    writable_roots: Vec<PathBuf>,
    inherited_environment: Vec<String>,
    network: SandboxNetworkPolicy,
    limits: SandboxResourceLimits,
}

impl SandboxLaunchProfile {
    /// Creates a deterministic profile after the caller has canonicalized filesystem paths.
    ///
    /// # Errors
    /// Returns an error for unsafe roots, ambient environment variables, invalid egress proof,
    /// or missing resource ceilings. This validates structure only; an OS backend must enforce it.
    pub fn new(
        workspace_root: &Path,
        readable_roots: &[PathBuf],
        writable_roots: &[PathBuf],
        inherited_environment: Vec<String>,
        network: SandboxNetworkPolicy,
        limits: SandboxResourceLimits,
    ) -> Result<Self, SandboxLaunchContractError> {
        let workspace_root = trusted_root(workspace_root)?;
        let readable_roots = trusted_roots(readable_roots, &workspace_root)?;
        let writable_roots = trusted_roots(writable_roots, &workspace_root)?;
        if !readable_roots.contains(&workspace_root)
            || writable_roots
                .iter()
                .any(|root| !readable_roots.iter().any(|read| root.starts_with(read)))
        {
            return Err(SandboxLaunchContractError::InvalidRoots);
        }
        Self::validate_policy(&inherited_environment, &network, limits)?;
        let inherited_environment = allowed_environment(inherited_environment)?;
        Ok(Self {
            workspace_root,
            readable_roots,
            writable_roots,
            inherited_environment,
            network,
            limits,
        })
    }

    /// Returns the stable SHA-256 identity a backend must bind to its prepared sandbox.
    #[must_use]
    pub fn digest_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        write_field(
            &mut hasher,
            "workspace",
            self.workspace_root.to_string_lossy().as_ref(),
        );
        for root in &self.readable_roots {
            write_field(&mut hasher, "read", root.to_string_lossy().as_ref());
        }
        for root in &self.writable_roots {
            write_field(&mut hasher, "write", root.to_string_lossy().as_ref());
        }
        for variable in &self.inherited_environment {
            write_field(&mut hasher, "env", variable);
        }
        match &self.network {
            SandboxNetworkPolicy::Disabled => write_field(&mut hasher, "network", "disabled"),
            SandboxNetworkPolicy::ReviewedEgress {
                policy_digest_sha256,
            } => write_field(&mut hasher, "egress", policy_digest_sha256),
        }
        write_field(
            &mut hasher,
            "processes",
            &self.limits.max_processes.to_string(),
        );
        write_field(
            &mut hasher,
            "memory",
            &self.limits.max_memory_bytes.to_string(),
        );
        write_field(&mut hasher, "cpu", &self.limits.max_cpu_time_ms.to_string());
        format!("{:x}", hasher.finalize())
    }

    /// Returns the network policy that a platform helper must enforce.
    #[must_use]
    pub const fn network(&self) -> &SandboxNetworkPolicy {
        &self.network
    }

    /// Returns the resource ceilings that a platform helper must enforce.
    #[must_use]
    pub const fn limits(&self) -> SandboxResourceLimits {
        self.limits
    }

    pub(crate) fn validate_policy(
        inherited_environment: &[String],
        network: &SandboxNetworkPolicy,
        limits: SandboxResourceLimits,
    ) -> Result<(), SandboxLaunchContractError> {
        allowed_environment(inherited_environment.to_vec())?;
        if let SandboxNetworkPolicy::ReviewedEgress {
            policy_digest_sha256,
        } = network
        {
            validate_digest(policy_digest_sha256)?;
        }
        if limits.max_processes == 0 || limits.max_memory_bytes == 0 || limits.max_cpu_time_ms == 0
        {
            return Err(SandboxLaunchContractError::MissingResourceLimit);
        }
        Ok(())
    }
}

/// Opaque launch request retained at the daemon/provider process boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxedLaunchRequest {
    pub lock: RunVersionLock,
    pub profile: SandboxLaunchProfile,
}

impl SandboxedLaunchRequest {
    /// Binds a backend result to a lock rediscovered immediately before sandbox preparation.
    ///
    /// # Errors
    /// Returns an error unless the rediscovered lock exactly matches the saved immutable lock and
    /// the backend reports verified enforcement. A provider process must not be spawned first.
    pub fn attest_after_lock_recheck(
        &self,
        rechecked_lock: &RunVersionLock,
        backend: SandboxBackendId,
        enforcement: SandboxEnforcement,
    ) -> Result<SandboxLaunchAttestation, SandboxLaunchContractError> {
        if enforcement != SandboxEnforcement::Enforced {
            return Err(SandboxLaunchContractError::NotEnforced);
        }
        if &self.lock != rechecked_lock || !valid_lock(&self.lock) {
            return Err(SandboxLaunchContractError::LockChanged);
        }
        Ok(SandboxLaunchAttestation {
            backend,
            profile_digest_sha256: self.profile.digest_sha256(),
            executable_digest_sha256: self.lock.digest_sha256.clone(),
            executable_file_identity: self.lock.file_identity.clone(),
        })
    }
}

/// Stable, non-secret identity of a platform-native enforcement backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxBackendId(String);

impl SandboxBackendId {
    /// Validates a bounded backend label without allowing paths, credentials, or endpoints.
    ///
    /// # Errors
    /// Returns an error unless the label uses only lowercase ASCII letters, digits, dots, or dashes.
    pub fn new(value: String) -> Result<Self, SandboxLaunchContractError> {
        if value.is_empty()
            || value.len() > 80
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
            })
        {
            return Err(SandboxLaunchContractError::InvalidBackend);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Trusted record returned only after sandbox setup and immutable-lock recheck succeed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxLaunchAttestation {
    pub backend: SandboxBackendId,
    pub profile_digest_sha256: String,
    pub executable_digest_sha256: String,
    pub executable_file_identity: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SandboxLaunchContractError {
    #[error("sandbox roots must be canonical absolute descendants of the workspace")]
    InvalidRoots,
    #[error("sandbox inherited environment is not an approved credential-free allowlist")]
    InvalidEnvironment,
    #[error("sandbox egress policy digest is invalid")]
    InvalidEgressPolicy,
    #[error("sandbox resource limits must all be non-zero")]
    MissingResourceLimit,
    #[error("sandbox backend identity is invalid")]
    InvalidBackend,
    #[error("provider executable changed before sandbox preparation")]
    LockChanged,
    #[error("sandbox backend did not verify containment")]
    NotEnforced,
}

fn trusted_roots(
    roots: &[PathBuf],
    workspace_root: &Path,
) -> Result<Vec<PathBuf>, SandboxLaunchContractError> {
    let mut roots = roots
        .iter()
        .map(|root| trusted_root(root))
        .collect::<Result<Vec<_>, _>>()?;
    if roots.iter().any(|root| !root.starts_with(workspace_root)) {
        return Err(SandboxLaunchContractError::InvalidRoots);
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn trusted_root(root: &Path) -> Result<PathBuf, SandboxLaunchContractError> {
    if !root.is_absolute()
        || root.parent().is_none()
        || root
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(SandboxLaunchContractError::InvalidRoots);
    }
    Ok(root.to_path_buf())
}

fn allowed_environment(mut values: Vec<String>) -> Result<Vec<String>, SandboxLaunchContractError> {
    const ALLOWED: &[&str] = &[
        "COLORTERM",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "NO_COLOR",
        "TERM",
        "TMP",
        "TMPDIR",
    ];
    if values
        .iter()
        .any(|value| !ALLOWED.contains(&value.as_str()))
    {
        return Err(SandboxLaunchContractError::InvalidEnvironment);
    }
    values.sort();
    values.dedup();
    Ok(values)
}

fn valid_lock(lock: &RunVersionLock) -> bool {
    !lock.provider.is_empty()
        && !lock.canonical_path.is_empty()
        && !lock.file_identity.is_empty()
        && !lock.version.is_empty()
        && !lock.compatibility_entry.is_empty()
        && validate_digest(&lock.digest_sha256).is_ok()
}

fn validate_digest(value: &str) -> Result<(), SandboxLaunchContractError> {
    (value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')))
    .then_some(())
    .ok_or(SandboxLaunchContractError::InvalidEgressPolicy)
}

fn write_field(hasher: &mut Sha256, name: &str, value: &str) {
    hasher.update(name.len().to_be_bytes());
    hasher.update(name.as_bytes());
    hasher.update(value.len().to_be_bytes());
    hasher.update(value.as_bytes());
}
