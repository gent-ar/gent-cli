//! Daemon-only sandbox values, never serializable or client-provided, for contained provider spawn.
use crate::{RunVersionLock, SandboxEnforcement, SandboxProviderGateway, SandboxResourceGrant};
use std::path::{Path, PathBuf};
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
    cpu_semantics: SandboxCpuSemantics,
    resources: Vec<SandboxResourceGrant>,
    gateway: Option<SandboxProviderGateway>,
    private_state: Option<SandboxPrivateState>,
    permission_binding: Option<crate::SandboxPermissionBinding>,
    shell_runtime: Option<SandboxShellRuntime>,
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
            cpu_semantics: SandboxCpuSemantics::StrictTotal,
            resources: Vec::new(),
            gateway: None,
            private_state: None,
            permission_binding: None,
            shell_runtime: None,
        })
    }

    pub fn with_resources(
        mut self,
        mut resources: Vec<SandboxResourceGrant>,
    ) -> Result<Self, SandboxLaunchContractError> {
        resources.sort();
        resources.dedup();
        for (index, grant) in resources.iter().enumerate() {
            let root = trusted_root(grant.root())?;
            if root.starts_with(&self.workspace_root)
                || self.workspace_root.starts_with(&root)
                || resources[..index]
                    .iter()
                    .any(|other| root.starts_with(other.root()) || other.root().starts_with(&root))
            {
                return Err(SandboxLaunchContractError::InvalidResourceGrant);
            }
        }
        if let Some(state) = &self.private_state
            && !resources.iter().any(|grant| {
                matches!(grant,
                SandboxResourceGrant::PrivateStateReadWrite(root) if root == state.root())
            })
        {
            return Err(SandboxLaunchContractError::InvalidResourceGrant);
        }
        self.resources = resources;
        if self
            .shell_runtime
            .as_ref()
            .is_some_and(|shell| !shell.covered_by(&self.resources))
        {
            return Err(SandboxLaunchContractError::InvalidResourceGrant);
        }
        Ok(self)
    }

    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    #[must_use]
    pub fn readable_roots(&self) -> &[PathBuf] {
        &self.readable_roots
    }

    #[must_use]
    pub fn writable_roots(&self) -> &[PathBuf] {
        &self.writable_roots
    }

    #[must_use]
    pub fn inherited_environment(&self) -> &[String] {
        &self.inherited_environment
    }

    #[must_use]
    pub fn resources(&self) -> &[SandboxResourceGrant] {
        &self.resources
    }

    pub fn with_gateway(
        mut self,
        gateway: SandboxProviderGateway,
    ) -> Result<Self, SandboxLaunchContractError> {
        if !matches!(&self.network, SandboxNetworkPolicy::ReviewedEgress { policy_digest_sha256 }
            if policy_digest_sha256 == gateway.policy_digest_sha256())
        {
            return Err(SandboxLaunchContractError::InvalidEgressPolicy);
        }
        self.gateway = Some(gateway);
        Ok(self)
    }

    pub fn gateway(&self) -> Option<&SandboxProviderGateway> {
        self.gateway.as_ref()
    }

    #[must_use]
    pub fn with_permission_binding(mut self, binding: crate::SandboxPermissionBinding) -> Self {
        self.permission_binding = Some(binding);
        self
    }

    pub fn permission_binding(&self) -> Option<&crate::SandboxPermissionBinding> {
        self.permission_binding.as_ref()
    }

    /// Returns the network policy that a platform helper must enforce.
    #[must_use]
    pub const fn network(&self) -> &SandboxNetworkPolicy {
        &self.network
    }

    /// Returns the resource ceilings that a platform helper must enforce.
    pub fn with_cpu_semantics(
        mut self,
        semantics: SandboxCpuSemantics,
    ) -> Result<Self, SandboxLaunchContractError> {
        self.cpu_semantics = semantics.validate()?;
        Ok(self)
    }
    pub const fn cpu_semantics(&self) -> SandboxCpuSemantics {
        self.cpu_semantics
    }
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
    #[error("sandbox resource grants overlap the workspace or another resource")]
    InvalidResourceGrant,
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
    #[error("the provider launch has no brokered command environment")]
    MissingCommandEgress,
}

#[path = "sandbox_launch_validation.rs"]
mod validation;
use validation::{allowed_environment, trusted_root, trusted_roots, valid_lock, validate_digest};

#[path = "sandbox_private_state.rs"]
mod private_state;
#[path = "sandbox_profile_digest.rs"]
mod profile_digest;
pub use private_state::SandboxPrivateState;
#[path = "sandbox_shell_runtime.rs"]
mod shell_runtime;
pub use shell_runtime::{SandboxCommandEgress, SandboxShellRuntime};
#[path = "sandbox_launch_values.rs"]
mod values;
pub use values::{SandboxBackendId, SandboxCpuSemantics};
