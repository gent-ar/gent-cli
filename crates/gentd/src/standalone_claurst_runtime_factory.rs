use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use gent_ports::{
    AgentChatWorkspaceLedger, ClaurstDrainBatch, ClaurstDrainRequest, ClaurstPermissionReply,
    ClaurstPromptAttachment, ClaurstSessionBinding, ClaurstStartRequest, ClaurstSubmitRequest,
    PortError, PrivateClaurstBridge, ToolSourceLedger,
};
use gent_runtime::AgentChatReadService;
use gent_runtime::conversation_summary_scheduler::ConversationSummaryScheduler;
use gent_store::SqliteLedger;
use gent_types::{AgentChatPromptSaved, AttachmentMetadata, PermissionMode};
use sha2::{Digest, Sha256};

use crate::{
    claurst_acp_bridge::ClaurstBridgeHandle,
    claurst_local_readiness::ClaurstLocalReadinessService,
    claurst_local_runtime::ClaurstLocalRuntimeRequest,
    claurst_local_runtime_owner::{
        HttpLlamaServerReadiness, SystemClaurstAcpStdio, SystemClaurstStandaloneLauncher,
        SystemLocalRuntimeProcess, SystemPrivateSettingsStore,
    },
    claurst_runtime_factory::ClaurstRuntimeFactory,
    claurst_standalone_owner::{ClaurstStandaloneOwner, ClaurstStandaloneRuntime},
};

type SystemRuntime = ClaurstStandaloneRuntime<SystemLocalRuntimeProcess, SystemClaurstAcpStdio>;
type SystemBridge = ClaurstBridgeHandle<SystemClaurstAcpStdio>;

struct ActiveRuntime {
    model_id: String,
    workspace: PathBuf,
    permission_mode: PermissionMode,
    mcp_config_digest: Option<String>,
    bridge: SystemBridge,
    runtime: SystemRuntime,
}

impl std::fmt::Debug for ActiveRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ActiveRuntime(..)")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StandaloneClaurstRuntimeConfig {
    pub(crate) request: ClaurstLocalRuntimeRequest,
    pub(crate) mcp_config: Option<crate::standalone_mcp_config::StandaloneMcpConfig>,
}

#[derive(Debug)]
pub(crate) struct StandaloneClaurstRuntimeFactory {
    ledger: SqliteLedger,
    readiness: ClaurstLocalReadinessService,
    models: crate::standalone_authority_composition::StandaloneClaurstModels,
    config: Option<StandaloneClaurstRuntimeConfig>,
    active: tokio::sync::Mutex<Option<ActiveRuntime>>,
}

#[derive(Clone, Debug)]
pub(crate) struct StandaloneClaurstBridge(Arc<StandaloneClaurstRuntimeFactory>);

impl StandaloneClaurstRuntimeFactory {
    #[must_use]
    pub(crate) fn new(
        ledger: SqliteLedger,
        models: crate::standalone_authority_composition::StandaloneClaurstModels,
        config: Option<StandaloneClaurstRuntimeConfig>,
    ) -> Self {
        Self {
            ledger,
            readiness: ClaurstLocalReadinessService::new(models.provisioner.clone()),
            models,
            config,
            active: tokio::sync::Mutex::new(None),
        }
    }

    #[must_use]
    pub(crate) fn bridge(self: &Arc<Self>) -> StandaloneClaurstBridge {
        StandaloneClaurstBridge(Arc::clone(self))
    }

    async fn active_bridge(&self) -> Result<SystemBridge, PortError> {
        self.active
            .lock()
            .await
            .as_ref()
            .map(|active| active.bridge.clone())
            .ok_or_else(|| PortError::Unavailable("local Claurst runtime is not ready".into()))
    }

    pub(crate) async fn summary_bridge(&self, model_id: &str) -> Result<SystemBridge, PortError> {
        let active = self.active.lock().await;
        let runtime = active
            .as_ref()
            .ok_or_else(|| PortError::Unavailable("local Claurst runtime is not ready".into()))?;
        if runtime.model_id != model_id {
            return Err(PortError::Unavailable(
                "selected Claurst model is not the active local model".into(),
            ));
        }
        if !runtime.bridge.is_idle().map_err(PortError::Unavailable)? {
            return Err(PortError::Unavailable(
                "Claurst summary waits for the interactive prompt to become idle".into(),
            ));
        }
        Ok(runtime.bridge.clone())
    }

    async fn start_selected(&self, saved: &AgentChatPromptSaved) -> Result<(), String> {
        let selection = AgentChatReadService::new(self.ledger.clone())
            .run_selection(&saved.message.conversation_id, &saved.run_id.0)
            .map_err(|error| error.to_string())?;
        let workspace_record = self
            .ledger
            .agent_chat_workspace_for_run(&saved.message.conversation_id, &saved.run_id.0)
            .map_err(|error| error.to_string())?;
        let workspace_id = workspace_record.workspace_id.clone();
        let workspace = PathBuf::from(workspace_record.canonical_path);
        let permission = crate::permission_workspace::policy_for(&self.ledger, &workspace_id)
            .map_err(|error| error.to_string())?;
        if !workspace.is_absolute() {
            return Err("the selected Gent workspace is not absolute".into());
        }

        let config = self.config.as_ref().ok_or_else(|| {
            "Claurst is selected but its local Claurst and llama.cpp executables are not installed"
                .to_owned()
        })?;
        let selected_sources = saved
            .tool_source_ids
            .iter()
            .map(|source_id| {
                self.ledger
                    .find_tool_source(source_id)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "selected MCP tool source does not exist".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if selected_sources.iter().any(|source| {
            source.workspace_id != workspace_id
                || source.kind != gent_types::ToolSourceKind::McpServer
        }) {
            return Err("selected MCP tool source is not available in this workspace".into());
        }
        let selected = !selected_sources.is_empty();
        let mcp_config_digest = config
            .mcp_config
            .as_ref()
            .map(crate::standalone_mcp_config::StandaloneMcpConfig::digest)
            .transpose()?;
        let permission_mode = if selection.mode == gent_types::AgentChatMode::Plan {
            PermissionMode::Plan
        } else {
            permission.mode
        };
        let previous = {
            let mut active = self.active.lock().await;
            if active.as_ref().is_some_and(|runtime| {
                runtime.model_id == selection.model
                    && runtime.workspace == workspace
                    && runtime.permission_mode == permission_mode
                    && runtime.mcp_config_digest == mcp_config_digest
            }) {
                return Ok(());
            }
            active.take()
        };
        if let Some(previous) = previous {
            let ActiveRuntime {
                bridge, runtime, ..
            } = previous;
            drop(bridge);
            tokio::task::spawn_blocking(move || runtime.shutdown())
                .await
                .map_err(|_| "local Claurst shutdown worker stopped unexpectedly".to_owned())?
                .map_err(|error| {
                    format!("could not stop previous local Claurst runtime: {error}")
                })?;
        }

        let model_id = selection.model;
        let mut request = config.request.clone();
        request.effort = selection.effort;
        request.mode = selection.mode;
        request.permission_mode = permission_mode;
        let settings_mcp_servers = config
            .mcp_config
            .as_ref()
            .map(|config| {
                if selected {
                    config.selected_claurst_settings_servers(&selected_sources)
                } else {
                    config.claurst_settings_servers()
                }
            })
            .transpose()?;
        request.mcp_servers = settings_mcp_servers.unwrap_or_default();
        let mcp_servers = config
            .mcp_config
            .as_ref()
            .map(|config| {
                if selected {
                    config.selected_claurst_servers(&selected_sources)
                } else {
                    config.claurst_servers()
                }
            })
            .transpose()?;
        let readiness = self.readiness.clone();
        let launch_model_id = model_id.clone();
        let launch_workspace = workspace.clone();
        let startup = tokio::task::spawn_blocking(move || {
            ClaurstStandaloneOwner::new(
                readiness,
                SystemPrivateSettingsStore,
                SystemClaurstStandaloneLauncher,
                HttpLlamaServerReadiness::default(),
            )
            .start_with_mcp(
                &launch_model_id,
                request,
                &launch_workspace,
                mcp_servers.unwrap_or_default(),
            )
        });
        let runtime = startup
            .await
            .map_err(|_| "local Claurst startup worker stopped unexpectedly".to_owned())?
            .map_err(|error| error.to_string())?;
        let bridge = ClaurstBridgeHandle::new(runtime.bridge());
        let mut active = self.active.lock().await;
        if active.is_some() {
            drop(bridge);
            let _ = tokio::task::spawn_blocking(move || runtime.shutdown()).await;
            return Err("local Claurst runtime changed while it was starting".into());
        }
        *active = Some(ActiveRuntime {
            model_id,
            workspace,
            permission_mode,
            mcp_config_digest,
            bridge,
            runtime,
        });
        Ok(())
    }

    async fn stop_active(&self) -> Result<(), String> {
        let previous = self.active.lock().await.take();
        if let Some(ActiveRuntime {
            bridge, runtime, ..
        }) = previous
        {
            drop(bridge);
            tokio::task::spawn_blocking(move || runtime.shutdown())
                .await
                .map_err(|_| "local Claurst shutdown worker stopped unexpectedly".to_owned())?
                .map_err(|error| format!("could not stop local Claurst runtime: {error}"))?;
        }
        Ok(())
    }
}

impl StandaloneClaurstBridge {
    pub(crate) async fn summary_bridge(&self, model_id: &str) -> Result<SystemBridge, PortError> {
        self.0.summary_bridge(model_id).await
    }
}

#[async_trait]
impl ClaurstRuntimeFactory for Arc<StandaloneClaurstRuntimeFactory> {
    async fn ensure_for_prompt(&self, saved: &AgentChatPromptSaved) -> Result<(), String> {
        self.start_selected(saved).await
    }

    async fn after_prompt_settled(&self, conversation_id: &str) -> Result<(), String> {
        if let Err(error) = ConversationSummaryScheduler::new(self.ledger.clone(), self.bridge())
            .schedule(conversation_id)
        {
            eprintln!("Claurst summary failed: {error}");
        }
        self.stop_active().await
    }

    async fn after_prompt_failed(&self, _: &str) -> Result<(), String> {
        self.stop_active().await
    }

    async fn prompt_attachments(
        &self,
        metadata: &[AttachmentMetadata],
    ) -> Result<Vec<ClaurstPromptAttachment>, String> {
        if metadata.is_empty() {
            return Ok(Vec::new());
        }
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| "Claurst local runtime is not configured".to_owned())?;
        let data_dir = config
            .request
            .claurst_home
            .parent()
            .ok_or_else(|| "Claurst local runtime has no data directory".to_owned())?;
        let blobs = gent_store::FileAttachmentBlobs::open(data_dir.join("attachments"))
            .map_err(|error| error.to_string())?;
        metadata
            .iter()
            .map(|attachment| {
                let bytes = gent_ports::AttachmentBlobStore::read_attachment_blob(
                    &blobs,
                    &attachment.storage_key,
                )
                .map_err(|error| error.to_string())?;
                let digest = format!("{:x}", Sha256::digest(&bytes));
                if bytes.len() as u64 != attachment.byte_len || digest != attachment.digest_sha256 {
                    return Err("Claurst attachment content failed its durable digest check".into());
                }
                Ok(ClaurstPromptAttachment {
                    display_name: attachment.display_name.clone(),
                    media_type: attachment.media_type.clone(),
                    bytes,
                })
            })
            .collect()
    }
}

#[async_trait]
impl PrivateClaurstBridge for StandaloneClaurstBridge {
    async fn start(
        &self,
        request: ClaurstStartRequest,
    ) -> Result<ClaurstSessionBinding, PortError> {
        self.0.active_bridge().await?.start(request).await
    }

    async fn bind_session(&self, binding: ClaurstSessionBinding) -> Result<(), PortError> {
        self.0.active_bridge().await?.bind_session(binding).await
    }

    async fn submit(&self, request: ClaurstSubmitRequest) -> Result<(), PortError> {
        self.0.active_bridge().await?.submit(request).await
    }

    async fn drain(&self, request: ClaurstDrainRequest) -> Result<ClaurstDrainBatch, PortError> {
        self.0.active_bridge().await?.drain(request).await
    }

    async fn respond_permission(
        &self,
        binding: ClaurstSessionBinding,
        request_id: &str,
        reply: ClaurstPermissionReply,
    ) -> Result<(), PortError> {
        self.0
            .active_bridge()
            .await?
            .respond_permission(binding, request_id, reply)
            .await
    }
}
