use super::{
    LocalProcessLaunch, Mutex, SandboxPermissionBinding, SqliteLedger, SystemLocalRuntimeProcess,
};
use crate::claurst_local_runtime_owner::{
    LocalRuntimeLauncher, LocalRuntimeProcess, SystemLocalRuntimeLauncher,
};
use crate::contained_model::ModelEndpoint;

pub(crate) enum RoutedModelProcess {
    Host(SystemLocalRuntimeProcess),
    Contained(ModelEndpoint),
}
impl LocalRuntimeProcess for RoutedModelProcess {
    fn contained_model(&self) -> Option<ModelEndpoint> {
        match self {
            Self::Contained(model) => Some(model.clone()),
            Self::Host(_) => None,
        }
    }
    fn exited(&mut self) -> Result<Option<String>, String> {
        match self {
            Self::Host(process) => process.exited(),
            Self::Contained(model) if model.current() => Ok(None),
            Self::Contained(model) if model.settled() => Ok(Some("contained model retired".into())),
            Self::Contained(_) => Err("contained model scope is unknown".into()),
        }
    }
    fn shutdown(&mut self) -> Result<(), String> {
        match self {
            Self::Host(process) => process.shutdown(),
            Self::Contained(model) => model.revoke().map_err(|e| e.to_string()),
        }
    }
}
impl Drop for RoutedModelProcess {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
pub(super) fn host(
    launch: &LocalProcessLaunch,
    contained: bool,
) -> Result<RoutedModelProcess, String> {
    if contained {
        return Err("contained model requires a catalog-bound launch plan".into());
    }
    SystemLocalRuntimeLauncher
        .launch(launch)
        .map(RoutedModelProcess::Host)
}
pub(super) fn launch(
    plan: &crate::claurst_local_runtime::ClaurstLocalRuntimePlan,
    binding: Option<&SandboxPermissionBinding>,
    ledger: Option<&SqliteLedger>,
    endpoint: &Mutex<Option<ModelEndpoint>>,
) -> Result<RoutedModelProcess, String> {
    let Some(binding) = binding else {
        return host(&plan.llama_server, false);
    };
    #[cfg(not(windows))]
    {
        let _ = (ledger, endpoint, binding);
        Err("native model containment unavailable".into())
    }
    #[cfg(windows)]
    {
        use crate::contained_model::{ModelRun, SelectedModel, WindowsModelFactory};
        use std::sync::Arc;
        let ledger = ledger.ok_or("Claurst model ledger missing")?.clone();
        let lock = gent_drivers::lock::capture(
            "claurst",
            &plan.claurst_acp.executable,
            "1",
            "claurst-acp-v1",
        )
        .map_err(|_| "Claurst image unavailable")?;
        let helper = crate::windows_sandbox_runtime::PinnedSandboxHelper::packaged()?;
        drop(crate::windows_claurst_resources::resources(&helper, &lock)?);
        let workspace = gent_ports::AgentChatWorkspaceLedger::agent_chat_workspace_for_run(
            &ledger,
            binding.conversation_id(),
            binding.run_id(),
        )
        .map_err(|_| "Claurst workspace missing")?;
        let sdk_lock = lock.clone();
        let coordinator =
            gent_runtime::Coordinator::new(ledger.clone(), gent_types::CapabilitySet::default());
        let fence = Arc::new(
            crate::provider_egress::run_fence::RunFence::capture(
                ledger,
                coordinator,
                binding.conversation_id().into(),
                binding.run_id().into(),
                lock,
            )
            .map_err(|_| "Claurst model run authority rejected")?,
        );
        if fence.policy != *binding.policy()
            || fence.host_epoch != binding.host_epoch()
            || fence.selection.model != plan.model_record.id
            || fence.selection.effort != plan.model_effort
            || fence.selection.mode != plan.model_mode
        {
            return Err("Claurst model permission binding changed".into());
        }
        let home = plan
            .settings_path
            .parent()
            .ok_or("Claurst model home missing")?;
        let writable_home = home
            .ancestors()
            .find(|path| path.is_dir())
            .ok_or("Claurst home ancestor missing")?;
        let parent = std::env::temp_dir().join("GentContainedModels");
        std::fs::create_dir_all(&parent).map_err(|_| "model scope parent unavailable")?;
        let run = ModelRun {
            binding: binding.clone(),
            sdk_lock,
            current: Arc::new(move || fence.current()),
            lifetime: Vec::new(),
            writable_roots: vec![writable_home.into(), workspace.canonical_path.into()],
        };
        let selected = SelectedModel::capture(plan.model_record.clone(), &plan.model_path)
            .map_err(|_| "Claurst selected model identity rejected")?;
        let model = WindowsModelFactory::packaged()
            .map_err(crate::contained_model::packaged_model_error)?
            .launch(selected, run, &parent, plan.model_configuration.clone())
            .map_err(|_| "Claurst contained model launch rejected")?;
        *endpoint
            .lock()
            .map_err(|_| "Claurst model lease unavailable")? = Some(model.clone());
        Ok(RoutedModelProcess::Contained(model))
    }
}

#[cfg(all(test, windows))]
#[path = "claurst_sandbox_model_launch_tests.rs"]
mod tests;
