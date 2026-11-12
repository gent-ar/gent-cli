use std::path::PathBuf;

use gent_drivers::supervisor::{ProviderLaunch, SupervisorError};
use gent_drivers::{ProcessLauncher, SystemLauncher};
use gent_ports::{AgentChatReadLedger, AgentChatWorkspaceLedger, Ledger};
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;
use gent_types::{SandboxPermissionBinding, SandboxWorkspaceAccess};

use crate::sandbox_provider_process::RoutedProviderProcess;

#[derive(Clone, Debug)]
pub(crate) struct SandboxProviderLauncher {
    host: SystemLauncher,
    ledger: SqliteLedger,
    #[cfg(any(windows, target_os = "linux"))]
    coordinator: Coordinator<SqliteLedger>,
    #[cfg(any(windows, target_os = "linux"))]
    state_root: PathBuf,
    #[cfg(windows)]
    output_limit: usize,
    managed_providers: Option<crate::provider_executables::ProviderExecutables>,
    lifecycle_control: Option<crate::ordinary_lifecycle_control::OrdinaryLifecycleControl>,
}

#[cfg(target_os = "linux")]
#[path = "sandbox_provider_launcher_linux.rs"]
mod linux;
impl SandboxProviderLauncher {
    pub(crate) fn new(
        host: SystemLauncher,
        ledger: SqliteLedger,
        coordinator: Coordinator<SqliteLedger>,
        state_root: PathBuf,
        output_limit: usize,
    ) -> Self {
        #[cfg(not(any(windows, target_os = "linux")))]
        let _ = (coordinator, state_root);
        #[cfg(not(windows))]
        let _ = output_limit;
        Self {
            host,
            ledger,
            #[cfg(any(windows, target_os = "linux"))]
            coordinator,
            #[cfg(any(windows, target_os = "linux"))]
            state_root,
            #[cfg(windows)]
            output_limit,
            managed_providers: None,
            lifecycle_control: None,
        }
    }

    pub(crate) fn with_managed_providers(
        mut self,
        providers: crate::provider_executables::ProviderExecutables,
    ) -> Self {
        self.managed_providers = Some(providers);
        self
    }
    pub(crate) fn with_lifecycle_control(
        mut self,
        control: crate::ordinary_lifecycle_control::OrdinaryLifecycleControl,
    ) -> Self {
        self.lifecycle_control = Some(control);
        self
    }

    fn current(
        &self,
        binding: &SandboxPermissionBinding,
    ) -> Result<gent_types::AgentChatProvider, SupervisorError> {
        let result = || {
            let detail = self
                .ledger
                .read_agent_chat_detail(binding.conversation_id())
                .ok()?;
            let workspace = self
                .ledger
                .agent_chat_workspace_for_run(binding.conversation_id(), binding.run_id())
                .ok()?;
            let ingress = self.ledger.host_ingress().ok()?;
            let provider = detail
                .runs
                .iter()
                .find(|run| run.run_id == binding.run_id())?
                .selection
                .provider;
            (workspace.workspace_id == binding.policy().workspace_id
                && ingress.epoch == binding.host_epoch()
                && ingress.mode == gent_ports::IngressMode::Open)
                .then_some(provider)
        };
        result().ok_or_else(|| SupervisorError::Launch("provider launch authority changed".into()))
    }

    #[cfg(windows)]
    fn contained(&self, launch: &ProviderLaunch) -> Result<RoutedProviderProcess, SupervisorError> {
        let binding = launch
            .permission_binding
            .as_ref()
            .ok_or_else(|| SupervisorError::Launch("provider run authority missing".into()))?;
        let helper = crate::windows_sandbox_runtime::PinnedSandboxHelper::packaged()
            .map_err(SupervisorError::Launch)?;
        let provider = gent_types::AgentChatProvider::from_contained_id(&launch.provider)
            .ok_or_else(|| {
                SupervisorError::Launch(
                    "contained provider credential contract is unavailable".into(),
                )
            })?;
        if self.current(binding)? != provider {
            return Err(SupervisorError::Launch(
                "provider launch does not match its run authority".into(),
            ));
        }
        helper
            .require_reviewed_policy(provider)
            .map_err(SupervisorError::Launch)?;
        let runtime = self
            .managed_providers
            .as_ref()
            .ok_or_else(|| {
                SupervisorError::Launch("managed provider factory is unavailable".into())
            })?
            .managed_runtime(provider, &launch.lock)
            .map_err(SupervisorError::Launch)?;
        let resources = crate::windows_provider_resources::managed(&helper, runtime)
            .map_err(SupervisorError::Launch)?;
        crate::permission_backend_registry::require_verified_launch(
            provider,
            binding,
            &launch.lock,
            &resources,
        )
        .map_err(SupervisorError::Launch)?;
        std::fs::create_dir_all(&self.state_root)
            .map_err(|_| SupervisorError::Launch("provider state directory unavailable".into()))?;
        let factory = match provider {
            gent_types::AgentChatProvider::Claude => {
                crate::provider_egress::ProviderGatewayLease::claude_managed
            }
            gent_types::AgentChatProvider::Codex => {
                crate::provider_egress::ProviderGatewayLease::codex_managed
            }
            gent_types::AgentChatProvider::Claurst => {
                return Err(SupervisorError::Launch(
                    "contained provider factory is unavailable".into(),
                ));
            }
        };
        let lease = factory(
            self.ledger.clone(),
            self.coordinator.clone(),
            binding,
            launch.lock.clone(),
            &self.state_root,
            &resources,
        )
        .map_err(|_| SupervisorError::Launch("provider gateway preparation rejected".into()))?;
        let launcher = lease
            .launcher_with_shell(
                helper.launcher(self.output_limit),
                gent_types::SandboxResourceLimits {
                    max_processes: 16,
                    max_memory_bytes: 2 * 1024 * 1024 * 1024,
                    max_cpu_time_ms: 300_000,
                },
                resources.grants(),
                Some(resources.shell()),
            )
            .map_err(|_| SupervisorError::Launch("provider sandbox profile rejected".into()))?
            .with_mcp_control(self.lifecycle_control.clone());
        // Resource pins must follow the same lease quarantine as private state
        // if a spawn or later scope settlement becomes unprovable.
        lease.retain_resource_handles(resources.into_handles());
        self.current(binding)?;
        launcher.launch(launch).map(RoutedProviderProcess::new)
    }
}

impl ProcessLauncher for SandboxProviderLauncher {
    type Process = RoutedProviderProcess;

    fn requires_sandbox_evidence(&self, launch: &ProviderLaunch) -> bool {
        launch
            .permission_binding
            .as_ref()
            .is_some_and(|binding| binding.policy().requires_sandbox())
    }

    fn validate_permission_binding(
        &self,
        binding: &SandboxPermissionBinding,
    ) -> Result<(), SupervisorError> {
        let provider = self.current(binding)?;
        if binding.policy().requires_sandbox() {
            #[cfg(windows)]
            return crate::windows_sandbox_runtime::PinnedSandboxHelper::packaged()
                .and_then(|helper| helper.require_reviewed_policy(provider))
                .map_err(SupervisorError::SandboxUnavailable);
            #[cfg(target_os = "linux")]
            return self
                .linux_authority(provider, None)?
                .require(provider)
                .map_err(SupervisorError::SandboxUnavailable);
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                let _ = provider;
                return Err(SupervisorError::SandboxUnavailable(
                    "native provider sandbox unavailable on this host".into(),
                ));
            }
        }
        Ok(())
    }

    fn launch(&self, launch: &ProviderLaunch) -> Result<Self::Process, SupervisorError> {
        let binding = launch
            .permission_binding
            .as_ref()
            .ok_or_else(|| SupervisorError::Launch("provider run authority missing".into()))?;
        self.validate_permission_binding(binding)?;
        let workspace = self
            .ledger
            .agent_chat_workspace_for_run(binding.conversation_id(), binding.run_id())
            .map_err(|_| SupervisorError::Launch("provider workspace missing".into()))?;
        let detail = self
            .ledger
            .read_agent_chat_detail(binding.conversation_id())
            .map_err(|_| SupervisorError::Launch("provider selection missing".into()))?;
        let selection = detail
            .runs
            .iter()
            .find(|run| run.run_id == binding.run_id())
            .ok_or_else(|| SupervisorError::Launch("provider selection missing".into()))?;
        if launch.run_id.as_deref() != Some(binding.run_id())
            || !matches!(
                (selection.selection.provider, launch.provider.as_str()),
                (gent_types::AgentChatProvider::Claude, "claude")
                    | (gent_types::AgentChatProvider::Codex, "codex")
            )
            || launch.workspace_root.as_deref()
                != Some(std::path::Path::new(&workspace.canonical_path))
            || launch.workspace_access
                != SandboxWorkspaceAccess::from_mode(selection.selection.mode)
        {
            return Err(SupervisorError::Launch(
                "provider launch binding changed".into(),
            ));
        }
        if binding.policy().requires_sandbox() {
            #[cfg(windows)]
            return self.contained(launch).map_err(admission);
            #[cfg(target_os = "linux")]
            return self.contained_linux(launch).map_err(admission);
            #[cfg(not(any(windows, target_os = "linux")))]
            return Err(SupervisorError::SandboxUnavailable(
                "native provider sandbox unavailable on this host".into(),
            ));
        }
        self.host.launch(launch).map(RoutedProviderProcess::new)
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn admission(error: SupervisorError) -> SupervisorError {
    match error {
        SupervisorError::Launch(reason) => SupervisorError::SandboxAdmission(reason),
        other => other,
    }
}

#[cfg(test)]
#[path = "sandbox_provider_launcher_tests.rs"]
mod tests;
