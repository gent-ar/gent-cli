use crate::claurst_local_runtime::LocalProcessLaunch;
use crate::claurst_local_runtime_owner::SystemClaurstStandaloneLauncher;
use crate::claurst_local_runtime_owner::SystemLocalRuntimeProcess;
use crate::claurst_sandbox_stdio::ClaurstOwnedAcpStdio;
use crate::claurst_standalone_owner::ClaurstStandaloneLauncher;
use gent_store::SqliteLedger;
use gent_types::{PermissionMode, SandboxPermissionBinding};
use std::sync::Mutex;

#[path = "claurst_sandbox_model_launch.rs"]
pub(crate) mod model_launch;
use model_launch::RoutedModelProcess;
#[cfg(test)]
#[path = "claurst_sandbox_route.rs"]
mod route;
#[path = "claurst_sandbox_settings.rs"]
mod settings;
use settings::{isolated_settings, packaged_shells};

pub(crate) struct RoutedClaurstStandaloneLauncher {
    ledger: Option<SqliteLedger>,
    binding: Option<SandboxPermissionBinding>,
    endpoint: Mutex<Option<crate::contained_model::ModelEndpoint>>,
    mcp_control: Option<crate::ordinary_lifecycle_control::OrdinaryLifecycleControl>,
}

impl RoutedClaurstStandaloneLauncher {
    pub(crate) fn host() -> Self {
        Self {
            ledger: None,
            binding: None,
            endpoint: Mutex::new(None),
            mcp_control: None,
        }
    }
    pub(crate) fn new(
        ledger: SqliteLedger,
        binding: Option<SandboxPermissionBinding>,
    ) -> Result<Self, String> {
        if let Some(binding) = &binding {
            crate::permission_backend_registry::require(
                gent_types::AgentChatProvider::Claurst,
                binding.policy().containment,
            )?;
        }
        Ok(Self {
            ledger: Some(ledger),
            binding,
            endpoint: Mutex::new(None),
            mcp_control: None,
        })
    }

    pub(crate) fn with_mcp_control(
        mut self,
        control: Option<crate::ordinary_lifecycle_control::OrdinaryLifecycleControl>,
    ) -> Self {
        self.mcp_control = control;
        self
    }
    #[cfg(windows)]
    fn contained(
        &self,
        launch: &LocalProcessLaunch,
        binding: &SandboxPermissionBinding,
    ) -> Result<ClaurstOwnedAcpStdio, String> {
        use gent_drivers::{
            ProcessLauncher,
            supervisor::{LaunchIntent, ProviderLaunch},
        };
        use std::path::PathBuf;
        let helper = crate::windows_sandbox_runtime::PinnedSandboxHelper::packaged()?;
        helper.require_reviewed_policy(gent_types::AgentChatProvider::Claurst)?;
        if launch.arguments != ["acp"] {
            return Err("Claurst contained command changed".into());
        }
        let lock =
            gent_drivers::lock::capture("claurst", &launch.executable, "1", "claurst-acp-v1")
                .map_err(|_| "Claurst image unavailable")?;
        let prepared = crate::windows_claurst_resources::resources(&helper, &lock)?;
        crate::permission_backend_registry::require_verified_launch(
            gent_types::AgentChatProvider::Claurst,
            binding,
            &lock,
            &prepared,
        )?;
        let resources = prepared.grants();
        let shell = prepared.shell();
        let (home, settings, contents, fixed_mcp_roster) = isolated_settings(launch)?;
        let state_root = home
            .parent()
            .ok_or("Claurst home has no parent")?
            .join("contained-runs");
        std::fs::create_dir_all(&state_root).map_err(|_| "Claurst state directory unavailable")?;
        let endpoint = self
            .endpoint
            .lock()
            .map_err(|_| "Claurst model lease unavailable")?
            .clone()
            .ok_or("Claurst model lease missing")?;
        let ledger = self
            .ledger
            .as_ref()
            .ok_or("Claurst authority ledger unavailable")?
            .clone();
        let model = endpoint.clone();
        let lease = crate::provider_egress::ProviderGatewayLease::claurst_contained(
            ledger.clone(),
            binding,
            lock.clone(),
            &state_root,
            endpoint,
        )
        .map_err(|_| "Claurst model gateway preparation rejected")?;
        lease.retain_resource_handles(prepared.into_handles());
        lease
            .prepare_claurst_mcp(&settings, &contents, self.mcp_control.as_ref())
            .map_err(|_| "Claurst MCP bridge preparation rejected")?;
        lease
            .materialize_claurst_settings(&contents)
            .map_err(|_| "Claurst contained settings rejected")?;
        let workspace = gent_ports::AgentChatWorkspaceLedger::agent_chat_workspace_for_run(
            &ledger,
            binding.conversation_id(),
            binding.run_id(),
        )
        .map_err(|_| "Claurst workspace missing")?;
        let detail = gent_ports::AgentChatReadLedger::read_agent_chat_detail(
            &ledger,
            binding.conversation_id(),
        )
        .map_err(|_| "Claurst selection missing")?;
        let run = detail
            .runs
            .iter()
            .find(|run| run.run_id == binding.run_id())
            .ok_or("Claurst selected run missing")?;
        let request = ProviderLaunch {
            permission_binding: Some(binding.clone()),
            run_id: Some(binding.run_id().into()),
            provider: "claurst".into(),
            executable: PathBuf::from(&lock.canonical_path),
            lock,
            arguments: launch.arguments.clone(),
            intent: LaunchIntent::Start,
            workspace_root: Some(workspace.canonical_path.into()),
            workspace_access: gent_types::SandboxWorkspaceAccess::from_mode(run.selection.mode),
        };
        let launcher = lease
            .launcher_with_shell(
                helper.launcher(gent_drivers::MAX_PROVIDER_FRAME_BYTES),
                gent_types::SandboxResourceLimits {
                    max_processes: 16,
                    max_memory_bytes: 2 * 1024 * 1024 * 1024,
                    max_cpu_time_ms: 300_000,
                },
                resources,
                Some(shell),
            )
            .map_err(|_| "Claurst sandbox profile rejected")?;
        let process = launcher
            .launch(&request)
            .map_err(|error| error.to_string())?;
        Ok(ClaurstOwnedAcpStdio::Contained {
            process: crate::sandbox_provider_process::RoutedProviderProcess::new(process),
            pending: Vec::new(),
            fixed_mcp_roster,
            model_current: Box::new(move || model.current()),
        })
    }
}

impl ClaurstStandaloneLauncher for RoutedClaurstStandaloneLauncher {
    type Llama = RoutedModelProcess;
    type Acp = ClaurstOwnedAcpStdio;

    fn validate_permission_mode(&self, mode: PermissionMode) -> Result<(), String> {
        let Some(binding) = self.binding.as_ref() else {
            return Ok(());
        };
        if binding.policy().mode != mode {
            return Err("Claurst run authority missing".into());
        }
        crate::permission_backend_registry::require(
            gent_types::AgentChatProvider::Claurst,
            binding.policy().containment,
        )
    }
    fn validate_acp_authority(
        &self,
        stdio: &mut Self::Acp,
        mode: PermissionMode,
    ) -> Result<(), String> {
        use crate::claurst_acp_transport::ClaurstAcpStdio;
        let Some(binding) = self.binding.as_ref() else {
            return Ok(());
        };
        if !matches!(stdio.exited(), Ok(None)) {
            return Err("contained Claurst process exited".into());
        }
        if binding.policy().mode != mode
            || !stdio.sandbox_evidence().is_some_and(|proof| {
                proof.run_id() == binding.run_id() && proof.permission_binding() == Some(binding)
            })
        {
            return Err("contained Claurst process evidence does not match this run".into());
        }
        Ok(())
    }
    fn launch_llama(&self, launch: &LocalProcessLaunch) -> Result<Self::Llama, String> {
        model_launch::host(launch, self.binding.is_some())
    }
    fn launch_model(
        &self,
        plan: &crate::claurst_local_runtime::ClaurstLocalRuntimePlan,
    ) -> Result<Self::Llama, String> {
        model_launch::launch(
            plan,
            self.binding.as_ref(),
            self.ledger.as_ref(),
            &self.endpoint,
        )
    }
    fn launch_acp(&self, launch: &LocalProcessLaunch) -> Result<Self::Acp, String> {
        if let Some(binding) = &self.binding {
            #[cfg(windows)]
            return self.contained(launch, binding);
            #[cfg(not(windows))]
            {
                let _ = binding;
                return Err("native Claurst containment unavailable on this host".into());
            }
        }
        let (_, _, _, fixed_mcp_roster) = isolated_settings(launch)?;
        let mut launch = launch.clone();
        launch.environment.extend(packaged_shells());
        SystemClaurstStandaloneLauncher
            .launch_acp(&launch)
            .map(|process| ClaurstOwnedAcpStdio::Host {
                process,
                fixed_mcp_roster,
            })
    }
}

#[cfg(test)]
#[path = "claurst_staged_release_gate_tests.rs"]
mod staged_release_gate_tests;
