//! Atomic containment-and-spawn boundary for public provider processes.
//!
//! A sandbox attestation alone is not authorization to use a separate launcher: it can become
//! stale between verification and `Command::spawn`. This port owns both operations in one
//! platform implementation. `SystemLauncher` deliberately does not implement this trait.

use std::sync::Arc;

use gent_types::{
    RunVersionLock, SandboxBackendId, SandboxEnforcement, SandboxLaunchAttestation,
    SandboxLaunchPolicy, SandboxProviderGateway, SandboxedLaunchRequest,
};

use crate::interrupt::ProcessTreeControl;
use crate::lock::LockError;
use crate::supervisor::{ProcessLauncher, ProviderLaunch, ProviderProcess, SupervisorError};

#[derive(Debug)]
pub struct ProcessSandboxEvidence {
    run_id: String,
    process_generation: String,
    attestation: SandboxLaunchAttestation,
    lock: RunVersionLock,
    network_peer: Option<crate::SandboxNetworkPeer>,
    permission_binding: Option<gent_types::SandboxPermissionBinding>,
    workspace_root: std::path::PathBuf,
    workspace_access: gent_types::SandboxWorkspaceAccess,
}

impl ProcessSandboxEvidence {
    pub fn new(
        request: &SandboxedLaunchRequest,
        rechecked_lock: &RunVersionLock,
        run_id: String,
        process_generation: String,
        backend: SandboxBackendId,
    ) -> Result<Self, SandboxedProviderLaunchError> {
        if run_id.trim().is_empty() || process_generation.trim().is_empty() {
            return Err(SandboxedProviderLaunchError::ContainmentRejected);
        }
        if request
            .profile
            .permission_binding()
            .is_some_and(|binding| binding.run_id() != run_id)
        {
            return Err(SandboxedProviderLaunchError::ContainmentRejected);
        }
        let attestation = request
            .attest_after_lock_recheck(rechecked_lock, backend, SandboxEnforcement::Enforced)
            .map_err(|_| SandboxedProviderLaunchError::LockChanged)?;
        Ok(Self {
            run_id,
            process_generation,
            attestation,
            lock: rechecked_lock.clone(),
            network_peer: None,
            permission_binding: request.profile.permission_binding().cloned(),
            workspace_root: request.profile.workspace_root().into(),
            workspace_access: if request
                .profile
                .writable_roots()
                .contains(&request.profile.workspace_root().to_path_buf())
            {
                gent_types::SandboxWorkspaceAccess::ReadWrite
            } else {
                gent_types::SandboxWorkspaceAccess::ReadOnly
            },
        })
    }

    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    #[must_use]
    pub fn process_generation(&self) -> &str {
        &self.process_generation
    }

    #[must_use]
    pub fn attestation(&self) -> &SandboxLaunchAttestation {
        &self.attestation
    }

    #[must_use]
    pub fn with_network_peer(mut self, peer: crate::SandboxNetworkPeer) -> Self {
        self.network_peer = Some(peer);
        self
    }

    pub fn network_peer(&self) -> Option<&crate::SandboxNetworkPeer> {
        self.network_peer.as_ref()
    }

    pub fn permission_binding(&self) -> Option<&gent_types::SandboxPermissionBinding> {
        self.permission_binding.as_ref()
    }

    #[must_use]
    pub(crate) fn matches_provider_launch(&self, launch: &ProviderLaunch) -> bool {
        launch.run_id.as_deref() == Some(self.run_id.as_str())
            && self.lock == launch.lock
            && launch.provider == self.lock.provider
            && launch.executable == std::path::Path::new(&self.lock.canonical_path)
            && launch.workspace_root.as_deref() == Some(self.workspace_root.as_path())
            && launch.workspace_access == self.workspace_access
            && self.permission_binding.is_some()
            && self.permission_binding.as_ref() == launch.permission_binding.as_ref()
    }

    pub fn matches(&self, request: &SandboxedLaunchRequest, run_id: &str) -> bool {
        self.run_id == run_id
            && self.permission_binding.as_ref() == request.profile.permission_binding()
            && self.network_peer.is_some() == request.profile.gateway().is_some()
            && match (request.profile.gateway(), self.network_peer.as_ref()) {
                (Some(gateway), Some(peer)) => peer.matches_active_gateway(gateway.peer_nonce()),
                (None, None) => true,
                _ => false,
            }
            && self.lock == request.lock
            && self.attestation.profile_digest_sha256 == request.profile.digest_sha256()
            && self.attestation.executable_digest_sha256 == request.lock.digest_sha256
            && self.attestation.executable_file_identity == request.lock.file_identity
    }
}

/// Failure from the platform-owned containment and spawn operation.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SandboxedProviderLaunchError {
    #[error("provider executable changed before contained launch")]
    LockChanged,
    #[error("a native sandbox backend is unavailable")]
    Unavailable,
    #[error("sandbox profile was rejected")]
    ProfileRejected,
    #[error("sandbox backend could not prove containment")]
    ContainmentRejected,
    #[error("contained provider launch failed: {0}")]
    LaunchFailed(String),
}

/// The only process-spawning port accepted by a sandbox-required authority.
///
/// Implementations must recheck `request.lock`, apply `request.profile`, and start the exact
pub trait SandboxedProviderLaunch: Send + Sync {
    /// The process tree owned after a successful contained spawn.
    type Process: ProviderProcess + 'static;

    /// Rechecks containment inputs and starts one contained provider process atomically.
    ///
    /// # Errors
    /// Returns without a provider process when containment or the immutable executable fence
    /// cannot be proved.
    fn launch_sandboxed(
        &self,
        request: &SandboxedLaunchRequest,
        launch: &ProviderLaunch,
    ) -> Result<Self::Process, SandboxedProviderLaunchError>;
}

/// A `ProcessLauncher` adapter that derives one sandbox request for every durable run.
///
/// It is the only launcher that dormant sandbox-required compositions may construct. The inner
/// port receives the profile and spawn request together, so it cannot attest and then delegate to
/// an unrelated `SystemLauncher`.
#[derive(Clone, Debug)]
pub struct SandboxedLauncher<S> {
    policy: SandboxLaunchPolicy,
    inner: Arc<S>,
    gateway: Option<(String, SandboxProviderGateway)>,
}

impl<S> SandboxedLauncher<S> {
    ///
    /// The exact executable lock comes from each daemon-resolved [`ProviderLaunch`], avoiding a
    /// second configuration lock that could diverge before contained spawning.
    #[must_use]
    pub fn new(policy: SandboxLaunchPolicy, inner: S) -> Self {
        Self {
            policy,
            inner: Arc::new(inner),
            gateway: None,
        }
    }

    pub fn with_gateway_for_run(
        mut self,
        run_id: String,
        gateway: SandboxProviderGateway,
    ) -> Result<Self, SandboxedProviderLaunchError> {
        if run_id.trim().is_empty() {
            return Err(SandboxedProviderLaunchError::ProfileRejected);
        }
        self.gateway = Some((run_id, gateway));
        Ok(self)
    }
}

impl<S> ProcessLauncher for SandboxedLauncher<S>
where
    S: SandboxedProviderLaunch + 'static,
{
    type Process = S::Process;

    fn launch(&self, launch: &ProviderLaunch) -> Result<Self::Process, SupervisorError> {
        let run_id = launch
            .run_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| {
                SupervisorError::Launch("sandboxed launch lacks a durable run".into())
            })?;
        let lock = &launch.lock;
        (launch.provider == lock.provider
            && launch.executable.to_string_lossy() == lock.canonical_path)
            .then_some(())
            .ok_or(SupervisorError::Lock(LockError::ProviderChanged))?;
        let workspace_root = launch
            .workspace_root
            .as_deref()
            .ok_or_else(|| SupervisorError::Launch("sandboxed launch lacks a workspace".into()))?;
        let mut profile = self
            .policy
            .profile_for_workspace(workspace_root, launch.workspace_access)
            .map_err(|_| SupervisorError::Launch("sandbox profile was rejected".into()))?;
        if profile.permission_binding() != launch.permission_binding.as_ref() {
            return Err(map_error(SandboxedProviderLaunchError::ProfileRejected));
        }
        if profile
            .private_state()
            .is_some_and(|state| state.run_id() != run_id)
        {
            return Err(map_error(SandboxedProviderLaunchError::ProfileRejected));
        }
        if let Some((gateway_run, gateway)) = &self.gateway {
            if gateway_run != run_id || launch.provider != gateway.api().provider_id() {
                return Err(map_error(SandboxedProviderLaunchError::ProfileRejected));
            }
            profile = profile
                .with_gateway(gateway.clone())
                .map_err(|_| map_error(SandboxedProviderLaunchError::ProfileRejected))?;
        }
        let request = SandboxedLaunchRequest {
            lock: lock.clone(),
            profile,
        };
        let process = self
            .inner
            .launch_sandboxed(&request, launch)
            .map_err(map_error)?;
        if !process
            .sandbox_evidence()
            .is_some_and(|proof| proof.matches(&request, run_id))
        {
            let _ = process.signal_tree(crate::interrupt::ProcessTreeSignal::Kill);
            return Err(map_error(SandboxedProviderLaunchError::ContainmentRejected));
        }
        Ok(process)
    }
}

fn map_error(error: SandboxedProviderLaunchError) -> SupervisorError {
    match error {
        SandboxedProviderLaunchError::LockChanged => {
            SupervisorError::Lock(LockError::ProviderChanged)
        }
        other => SupervisorError::Launch(other.to_string()),
    }
}

#[cfg(test)]
#[path = "sandboxed_launch_tests.rs"]
mod tests;
