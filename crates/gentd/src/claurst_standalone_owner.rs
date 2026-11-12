use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use crate::{
    claurst_acp_bridge::ClaurstAcpBridge,
    claurst_acp_transport::ClaurstAcpStdio,
    claurst_local_readiness::{
        ClaurstLocalReadiness, ClaurstLocalReadinessError, ClaurstLocalReadinessService,
    },
    claurst_local_runtime::{ClaurstLocalRuntimeRequest, LocalProcessLaunch},
    claurst_local_runtime_owner::{
        LlamaServerReadiness, LocalRuntimeProcess, PrivateSettingsStore,
    },
    claurst_runtime_factory::LlamaContextSummarizer,
    local_model_provisioning::LocalModelDownloadPlan,
};

pub(crate) trait ClaurstStandaloneLauncher {
    type Llama: LocalRuntimeProcess;
    type Acp: ClaurstAcpStdio + Send;

    fn validate_permission_mode(&self, mode: gent_types::PermissionMode) -> Result<(), String> {
        if mode.is_unattended() {
            Err("contained Claurst launcher unavailable".into())
        } else {
            Ok(())
        }
    }
    fn validate_acp_authority(
        &self,
        _stdio: &mut Self::Acp,
        mode: gent_types::PermissionMode,
    ) -> Result<(), String> {
        if mode.is_unattended() {
            Err("live contained Claurst authority missing".into())
        } else {
            Ok(())
        }
    }
    fn launch_llama(&self, launch: &LocalProcessLaunch) -> Result<Self::Llama, String>;
    fn launch_model(
        &self,
        plan: &crate::claurst_local_runtime::ClaurstLocalRuntimePlan,
    ) -> Result<Self::Llama, String> {
        self.launch_llama(&plan.llama_server)
    }
    fn launch_acp(&self, launch: &LocalProcessLaunch) -> Result<Self::Acp, String>;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ClaurstStandaloneStartError {
    #[error(transparent)]
    Readiness(#[from] ClaurstLocalReadinessError),
    #[error("selected Claurst model `{model_id}` is not installed")]
    DownloadRequired {
        model_id: String,
        plan: Box<LocalModelDownloadPlan>,
        downloaded_bytes: u64,
    },
    #[error("could not materialize isolated Claurst settings: {0}")]
    Settings(String),
    #[error("could not start local llama.cpp: {0}")]
    Llama(String),
    #[error("local llama.cpp did not become ready: {0}")]
    ReadinessProbe(String),
    #[error("could not start Claurst ACP: {0}")]
    Acp(String),
    #[error("the local llama.cpp plan changed, so its model must be reloaded")]
    ModelPlanChanged,
    #[error("selected permission mode requires verified OS containment for this Claurst run")]
    SandboxRequired,
}

pub(crate) struct ClaurstStandaloneOwner<S, L, R> {
    readiness: ClaurstLocalReadinessService,
    settings: S,
    launcher: L,
    probe: R,
}

impl<S, L, R> ClaurstStandaloneOwner<S, L, R>
where
    S: PrivateSettingsStore,
    L: ClaurstStandaloneLauncher,
    R: LlamaServerReadiness,
{
    #[must_use]
    pub(crate) fn new(
        readiness: ClaurstLocalReadinessService,
        settings: S,
        launcher: L,
        probe: R,
    ) -> Self {
        Self {
            readiness,
            settings,
            launcher,
            probe,
        }
    }

    pub(crate) fn start(
        &self,
        model_id: &str,
        request: ClaurstLocalRuntimeRequest,
        workspace: &Path,
    ) -> Result<ClaurstStandaloneRuntime<L::Llama, L::Acp>, ClaurstStandaloneStartError> {
        self.start_with_mcp(model_id, request, workspace, Vec::new())
    }

    pub(crate) fn attach_session(
        &self,
        model: &ClaurstStandaloneModel<L::Llama>,
        model_id: &str,
        request: ClaurstLocalRuntimeRequest,
        workspace: &Path,
        mcp_servers: Vec<serde_json::Value>,
    ) -> Result<ClaurstStandaloneRuntime<L::Llama, L::Acp>, ClaurstStandaloneStartError> {
        let mode = request.permission_mode;
        let plan = self.ready_plan(model_id, request, model.port, workspace)?;
        if plan.llama_server != model.llama_launch {
            return Err(ClaurstStandaloneStartError::ModelPlanChanged);
        }
        Ok(ClaurstStandaloneRuntime {
            model: model.clone(),
            bridge: Arc::new(
                ClaurstAcpBridge::new(
                    workspace.to_path_buf(),
                    self.materialize_and_launch_acp(&plan, workspace, mode)?,
                    mcp_servers,
                )
                .with_history_input_bytes(plan.history_input_bytes),
            ),
        })
    }

    fn ready_plan(
        &self,
        model_id: &str,
        request: ClaurstLocalRuntimeRequest,
        port: u16,
        workspace: &Path,
    ) -> Result<crate::claurst_local_runtime::ClaurstLocalRuntimePlan, ClaurstStandaloneStartError>
    {
        self.launcher
            .validate_permission_mode(request.permission_mode)
            .map_err(|_| ClaurstStandaloneStartError::SandboxRequired)?;
        let plan = match self.readiness.assess(model_id, request, port)? {
            ClaurstLocalReadiness::DownloadRequired {
                plan,
                downloaded_bytes,
            } => {
                return Err(ClaurstStandaloneStartError::DownloadRequired {
                    model_id: model_id.into(),
                    plan: Box::new(plan),
                    downloaded_bytes,
                });
            }
            ClaurstLocalReadiness::Ready(plan) => *plan,
        };
        if !workspace.is_absolute() {
            return Err(ClaurstStandaloneStartError::Acp(
                "Gent workspace must be absolute".into(),
            ));
        }
        Ok(plan)
    }

    fn materialize_chat_template(
        &self,
        plan: &crate::claurst_local_runtime::ClaurstLocalRuntimePlan,
    ) -> Result<(), ClaurstStandaloneStartError> {
        if let (Some(path), Some(contents)) =
            (&plan.chat_template_path, &plan.chat_template_contents)
        {
            self.settings
                .materialize(path, contents)
                .map_err(ClaurstStandaloneStartError::Settings)?;
        }
        Ok(())
    }

    fn materialize_and_launch_acp(
        &self,
        plan: &crate::claurst_local_runtime::ClaurstLocalRuntimePlan,
        workspace: &Path,
        mode: gent_types::PermissionMode,
    ) -> Result<L::Acp, ClaurstStandaloneStartError> {
        self.settings
            .materialize(&plan.settings_path, &plan.settings_json)
            .map_err(ClaurstStandaloneStartError::Settings)?;
        let mut acp = self
            .launcher
            .launch_acp(&LocalProcessLaunch {
                working_directory: Some(workspace.to_path_buf()),
                ..plan.claurst_acp.clone()
            })
            .map_err(ClaurstStandaloneStartError::Acp)?;
        self.launcher
            .validate_acp_authority(&mut acp, mode)
            .map_err(ClaurstStandaloneStartError::Acp)?;
        Ok(acp)
    }

    pub(crate) fn start_with_mcp(
        &self,
        model_id: &str,
        request: ClaurstLocalRuntimeRequest,
        workspace: &Path,
        mcp_servers: Vec<serde_json::Value>,
    ) -> Result<ClaurstStandaloneRuntime<L::Llama, L::Acp>, ClaurstStandaloneStartError> {
        let mode = request.permission_mode;
        let port = select_ephemeral_loopback_port().map_err(ClaurstStandaloneStartError::Llama)?;
        let plan = self.ready_plan(model_id, request, port, workspace)?;
        self.materialize_chat_template(&plan)?;
        let mut llama = self
            .launcher
            .launch_model(&plan)
            .map_err(ClaurstStandaloneStartError::Llama)?;
        if let Err(error) = self.probe.wait_ready(&plan.server_url, &mut llama) {
            let _ = llama.shutdown();
            return Err(ClaurstStandaloneStartError::ReadinessProbe(error));
        }
        let acp = match self.materialize_and_launch_acp(&plan, workspace, mode) {
            Ok(acp) => acp,
            Err(error) => {
                let _ = llama.shutdown();
                return Err(error);
            }
        };
        Ok(ClaurstStandaloneRuntime {
            model: ClaurstStandaloneModel {
                summarizer: Arc::new(
                    LlamaContextSummarizer::new(plan.summary_endpoint.clone())
                        .with_contained_model(llama.contained_model()),
                ),
                llama: Arc::new(Mutex::new(llama)),
                port,
                llama_launch: plan.llama_server.clone(),
            },
            bridge: Arc::new(
                ClaurstAcpBridge::new(workspace.to_path_buf(), acp, mcp_servers)
                    .with_history_input_bytes(plan.history_input_bytes),
            ),
        })
    }
}

#[path = "claurst_standalone_owner_port.rs"]
mod port_selection;
use port_selection::select_ephemeral_loopback_port;

#[cfg(test)]
#[path = "claurst_standalone_owner_tests.rs"]
mod tests;

#[path = "claurst_standalone_owner_runtime.rs"]
mod runtime;
pub(crate) use runtime::{ClaurstStandaloneModel, ClaurstStandaloneRuntime};
