use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    sync::Arc,
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
    local_model_download::{
        ModelDownloadError, ModelDownloadProgress, ModelDownloadTransport, download_model,
    },
    local_model_provisioning::LocalModelDownloadPlan,
};

pub(crate) trait ClaurstStandaloneLauncher {
    type Llama: LocalRuntimeProcess;
    type Acp: ClaurstAcpStdio + Send;

    fn launch_llama(&self, launch: &LocalProcessLaunch) -> Result<Self::Llama, String>;
    fn launch_acp(&self, launch: &LocalProcessLaunch) -> Result<Self::Acp, String>;
}

pub(crate) struct ClaurstStandaloneRuntime<L, S> {
    llama: L,
    bridge: Arc<ClaurstAcpBridge<S>>,
}

impl<L, S> ClaurstStandaloneRuntime<L, S> {
    #[must_use]
    pub(crate) fn bridge(&self) -> Arc<ClaurstAcpBridge<S>> {
        Arc::clone(&self.bridge)
    }
}

impl<L: LocalRuntimeProcess, S> ClaurstStandaloneRuntime<L, S> {
    pub(crate) fn shutdown(self) -> Result<(), String> {
        drop(self.bridge);
        let mut llama = self.llama;
        llama.shutdown()
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ClaurstStandaloneStartError {
    #[error(transparent)]
    Readiness(#[from] ClaurstLocalReadinessError),
    #[error("selected Claurst model `{model_id}` is not installed")]
    DownloadRequired {
        model_id: String,
        plan: LocalModelDownloadPlan,
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

    pub(crate) async fn download_after_consent(
        plan: &LocalModelDownloadPlan,
        transport: &dyn ModelDownloadTransport,
        report: impl FnMut(ModelDownloadProgress),
    ) -> Result<PathBuf, ModelDownloadError> {
        download_model(plan, transport, report).await
    }

    pub(crate) fn start(
        &self,
        model_id: &str,
        request: ClaurstLocalRuntimeRequest,
        workspace: &Path,
    ) -> Result<ClaurstStandaloneRuntime<L::Llama, L::Acp>, ClaurstStandaloneStartError> {
        self.start_with_mcp(model_id, request, workspace, Vec::new())
    }

    pub(crate) fn start_with_mcp(
        &self,
        model_id: &str,
        request: ClaurstLocalRuntimeRequest,
        workspace: &Path,
        mcp_servers: Vec<serde_json::Value>,
    ) -> Result<ClaurstStandaloneRuntime<L::Llama, L::Acp>, ClaurstStandaloneStartError> {
        let port = select_ephemeral_loopback_port().map_err(ClaurstStandaloneStartError::Llama)?;
        let plan = match self.readiness.assess(model_id, request, port)? {
            ClaurstLocalReadiness::DownloadRequired {
                plan,
                downloaded_bytes,
            } => {
                return Err(ClaurstStandaloneStartError::DownloadRequired {
                    model_id: model_id.into(),
                    plan,
                    downloaded_bytes,
                });
            }
            ClaurstLocalReadiness::Ready(plan) => plan,
        };
        if !workspace.is_absolute() {
            return Err(ClaurstStandaloneStartError::Acp(
                "Gent workspace must be absolute".into(),
            ));
        }
        self.settings
            .materialize(&plan.settings_path, &plan.settings_json)
            .map_err(ClaurstStandaloneStartError::Settings)?;
        if let (Some(path), Some(contents)) =
            (&plan.chat_template_path, &plan.chat_template_contents)
        {
            self.settings
                .materialize(path, contents)
                .map_err(ClaurstStandaloneStartError::Settings)?;
        }
        let mut llama = self
            .launcher
            .launch_llama(&plan.llama_server)
            .map_err(ClaurstStandaloneStartError::Llama)?;
        if let Err(error) = self.probe.wait_ready(&plan.server_url, &mut llama) {
            let _ = llama.shutdown();
            return Err(ClaurstStandaloneStartError::ReadinessProbe(error));
        }
        let acp = match self.launcher.launch_acp(&plan.claurst_acp) {
            Ok(acp) => acp,
            Err(error) => {
                let _ = llama.shutdown();
                return Err(ClaurstStandaloneStartError::Acp(error));
            }
        };
        Ok(ClaurstStandaloneRuntime {
            llama,
            bridge: Arc::new(ClaurstAcpBridge::new(
                workspace.to_path_buf(),
                acp,
                mcp_servers,
            )),
        })
    }
}

fn select_ephemeral_loopback_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    drop(listener);
    Ok(port)
}

#[cfg(test)]
#[path = "claurst_standalone_owner_tests.rs"]
mod tests;
