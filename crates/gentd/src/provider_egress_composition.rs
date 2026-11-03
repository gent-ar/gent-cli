use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use gent_drivers::{ProcessLauncher, SandboxedLauncher, SandboxedProviderLaunch};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{
    RunVersionLock, SandboxLaunchPolicy, SandboxNetworkPolicy, SandboxPrivateState,
    SandboxResourceGrant, SandboxResourceLimits,
};

use super::{
    EgressError, ProviderApi, ProviderEgressBroker, ReviewedProviderEgress, claude_credential,
    run_fence::RunFence,
};

#[cfg(windows)]
#[path = "provider_egress_codex_composition.rs"]
mod codex_composition;

pub struct ProviderGatewayLease {
    pub(super) fence: Arc<RunFence>,
    pub(super) broker: ProviderEgressBroker,
    revoked: AtomicBool,
    pub(super) private_state: SandboxPrivateState,
    _private_root: tempfile::TempDir,
    retained_resources: std::sync::Mutex<Vec<std::fs::File>>,
    contained_model: Option<crate::contained_model::ModelEndpoint>,
    #[cfg(any(windows, target_os = "linux"))]
    pub(super) mcp: std::sync::Mutex<Option<Arc<super::mcp_owner::McpOwner>>>,
    #[cfg(target_os = "linux")]
    pub(super) mcp_deployment: std::sync::Mutex<Option<super::mcp_owner::LinuxMcpDeployment>>,
}

impl std::fmt::Debug for ProviderGatewayLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderGatewayLease")
            .field("run_id", &self.fence.run_id)
            .finish_non_exhaustive()
    }
}

impl ProviderGatewayLease {
    #[cfg(all(test, windows))]
    pub fn claude(
        ledger: SqliteLedger,
        coordinator: Coordinator<SqliteLedger>,
        conversation_id: String,
        run_id: String,
        lock: RunVersionLock,
        probe_root: &Path,
    ) -> Result<Arc<Self>, EgressError> {
        Self::claude_using(
            ledger,
            coordinator,
            conversation_id,
            run_id,
            lock,
            probe_root,
            |lock| claude_credential::resolve(lock, probe_root),
        )
    }

    #[cfg(windows)]
    pub(crate) fn claude_managed(
        ledger: SqliteLedger,
        coordinator: Coordinator<SqliteLedger>,
        conversation_id: String,
        run_id: String,
        lock: RunVersionLock,
        probe_root: &Path,
        resources: &crate::windows_provider_resources::ManagedProviderResources,
    ) -> Result<Arc<Self>, EgressError> {
        Self::claude_using(
            ledger,
            coordinator,
            conversation_id,
            run_id,
            lock,
            probe_root,
            |lock| claude_credential::resolve_managed(lock, probe_root, resources),
        )
    }

    fn claude_using(
        ledger: SqliteLedger,
        coordinator: Coordinator<SqliteLedger>,
        conversation_id: String,
        run_id: String,
        lock: RunVersionLock,
        probe_root: &Path,
        credential: impl FnOnce(
            &RunVersionLock,
        )
            -> Result<super::credential_capture::CapturedProviderAuth, EgressError>,
    ) -> Result<Arc<Self>, EgressError> {
        let fence = RunFence::capture(ledger, coordinator, conversation_id, run_id, lock)?;
        if fence.lock.provider != "claude" || !fence.current() {
            return Err(EgressError::Configuration);
        }
        let private_root = tempfile::Builder::new()
            .prefix("provider-state-")
            .tempdir_in(probe_root)
            .map_err(|_| EgressError::Configuration)?;
        let root =
            std::fs::canonicalize(private_root.path()).map_err(|_| EgressError::Configuration)?;
        let private_state = SandboxPrivateState::new(fence.run_id.clone(), root)
            .map_err(|_| EgressError::Configuration)?;
        for directory in private_state.directories() {
            std::fs::create_dir(&directory).map_err(|_| EgressError::Configuration)?;
        }
        let credential = credential(&fence.lock)?;
        if !fence.current() {
            return Err(EgressError::Configuration);
        }
        let policy = ReviewedProviderEgress::new(
            ProviderApi::Claude,
            "https://api.anthropic.com/v1/",
            credential.headers,
        )?;
        let fence = Arc::new(fence);
        let current = fence.clone();
        let broker =
            ProviderEgressBroker::start_checked(policy, Arc::new(move || current.current()))?;
        Ok(Arc::new(Self {
            fence,
            broker,
            retained_resources: std::sync::Mutex::new(Vec::new()),
            revoked: AtomicBool::new(false),
            private_state,
            _private_root: private_root,
            contained_model: None,
            #[cfg(any(windows, target_os = "linux"))]
            mcp: std::sync::Mutex::default(),
            #[cfg(target_os = "linux")]
            mcp_deployment: std::sync::Mutex::default(),
        }))
    }

    #[cfg(all(test, windows))]
    pub fn launcher<S: SandboxedProviderLaunch + 'static>(
        self: &Arc<Self>,
        backend: S,
        limits: SandboxResourceLimits,
        resources: Vec<SandboxResourceGrant>,
    ) -> Result<GatewayBoundLauncher<S>, EgressError> {
        self.launcher_with_shell(backend, limits, resources, None)
    }

    #[cfg(windows)]
    pub fn launcher_with_shell<S: SandboxedProviderLaunch + 'static>(
        self: &Arc<Self>,
        backend: S,
        limits: SandboxResourceLimits,
        mut resources: Vec<SandboxResourceGrant>,
        shell: Option<gent_types::SandboxShellRuntime>,
    ) -> Result<GatewayBoundLauncher<S>, EgressError> {
        if !self.current() {
            return Err(EgressError::Configuration);
        }
        resources.push(SandboxResourceGrant::PrivateStateReadWrite(
            self.private_state.root().to_path_buf(),
        ));
        let mut policy = SandboxLaunchPolicy::new(
            vec![],
            SandboxNetworkPolicy::ReviewedEgress {
                policy_digest_sha256: self.broker.policy_digest().into(),
            },
            limits,
        )
        .map_err(|_| EgressError::Configuration)?
        .with_resources(resources)
        .with_private_state(self.private_state.clone())
        .with_permission_binding(
            gent_types::SandboxPermissionBinding::new(
                self.fence.conversation_id.clone(),
                self.fence.run_id.clone(),
                self.fence.policy.clone(),
                self.fence.host_epoch,
            )
            .map_err(|_| EgressError::Configuration)?,
        );
        if let Some(shell) = shell {
            policy = policy.with_shell_runtime(shell);
        }
        let launcher = SandboxedLauncher::new(policy, backend)
            .with_gateway_for_run(self.fence.run_id.clone(), self.broker.gateway_binding()?)
            .map_err(|_| EgressError::Configuration)?;
        Ok(GatewayBoundLauncher {
            launcher,
            lease: self.clone(),
            #[cfg(any(windows, target_os = "linux"))]
            mcp_control: None,
        })
    }

    pub(super) fn current(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
            && self.fence.current()
            && self
                .contained_model
                .as_ref()
                .is_none_or(|model| model.current())
    }

    pub(crate) fn retain_resource_handles(&self, handles: Vec<std::fs::File>) {
        self.retained_resources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(handles);
    }

    pub(super) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        self.broker.revoke();
        if let Some(model) = &self.contained_model {
            let _ = model.revoke();
        }
    }
    pub(super) fn mcp_settled(&self) -> bool {
        self.broker.mcp_settled()
            && self
                .contained_model
                .as_ref()
                .is_none_or(|model| model.settled())
    }
}

#[path = "provider_egress_launcher.rs"]
mod bound_launcher;
pub use bound_launcher::GatewayBoundLauncher;

#[cfg(windows)]
#[path = "provider_egress_claurst_composition.rs"]
mod claurst;

#[cfg(all(test, windows))]
#[path = "provider_egress_lifetime_tests.rs"]
mod lifetime_tests;

#[cfg(all(test, windows))]
#[path = "provider_egress_contained_model_tests.rs"]
mod contained_model_tests;

#[cfg(target_os = "linux")]
#[path = "provider_egress_linux_composition.rs"]
mod linux_composition;
#[cfg(target_os = "linux")]
pub(crate) use linux_composition::{LinuxCredentialDeployment, LinuxRuntimeDeployment};
