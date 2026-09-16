use std::path::PathBuf;

use gent_ports::{AgentChatWorkspaceLedger, ToolSourceLedger};
use gent_runtime::AgentChatReadService;
use gent_types::{AgentChatPromptSaved, ToolSourceRecord};

use super::{ActiveRuntime, StandaloneClaurstRuntimeFactory};
use crate::standalone_claurst_runtime_identity::{RuntimeIdentity, RuntimeReuse};
use crate::standalone_mcp_config::StandaloneMcpConfig;
use crate::{
    claurst_acp_bridge::ClaurstBridgeHandle,
    claurst_local_runtime_owner::{
        HttpLlamaServerReadiness, SystemClaurstStandaloneLauncher, SystemPrivateSettingsStore,
    },
    claurst_standalone_owner::ClaurstStandaloneOwner,
};

impl StandaloneClaurstRuntimeFactory {
    pub(super) async fn start_selected(&self, saved: &AgentChatPromptSaved) -> Result<(), String> {
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
        let selected_sources = self.selected_tool_sources(saved, &workspace_id)?;
        let mcp_config_digest = config
            .mcp_config
            .as_ref()
            .map(crate::standalone_mcp_config::StandaloneMcpConfig::digest)
            .transpose()?;
        let permission_mode = permission.mode;
        let conversation_id = saved.message.conversation_id.clone();
        let identity = RuntimeIdentity::new(
            &selection,
            conversation_id.clone(),
            workspace.clone(),
            permission_mode,
            mcp_config_digest,
            &saved.tool_source_ids,
        );
        let reuse = self.retire_unless_reusable(&identity).await?;
        if reuse == RuntimeReuse::Ready {
            return Ok(());
        }
        let mut request = config.request.clone();
        request.effort = selection.effort;
        request.mode = selection.mode;
        request.permission_mode = permission_mode;
        request.mcp_servers = conversation_servers(
            config.mcp_config.as_ref(),
            &selected_sources,
            &conversation_id,
            StandaloneMcpConfig::claurst_settings_servers,
            StandaloneMcpConfig::selected_claurst_settings_servers,
        )?;
        let launch_mcp_servers = conversation_servers(
            config.mcp_config.as_ref(),
            &selected_sources,
            &conversation_id,
            StandaloneMcpConfig::claurst_servers,
            StandaloneMcpConfig::selected_claurst_servers,
        )?;
        match reuse {
            RuntimeReuse::Ready => Ok(()),
            RuntimeReuse::RebindSession
                if self
                    .rebind_session(
                        &identity,
                        request.clone(),
                        &workspace,
                        launch_mcp_servers.clone(),
                    )
                    .await? =>
            {
                Ok(())
            }
            RuntimeReuse::RebindSession | RuntimeReuse::Relaunch => {
                self.launch_runtime(identity, request, workspace, launch_mcp_servers)
                    .await
            }
        }
    }

    async fn launch_runtime(
        &self,
        identity: RuntimeIdentity,
        request: crate::claurst_local_runtime::ClaurstLocalRuntimeRequest,
        workspace: PathBuf,
        mcp_servers: Vec<serde_json::Value>,
    ) -> Result<(), String> {
        let readiness = self.readiness.clone();
        let model_id = identity.model.model_id.clone();
        let runtime = tokio::task::spawn_blocking(move || {
            ClaurstStandaloneOwner::new(
                readiness,
                SystemPrivateSettingsStore,
                SystemClaurstStandaloneLauncher,
                HttpLlamaServerReadiness::default(),
            )
            .start_with_mcp(&model_id, request, &workspace, mcp_servers)
        })
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
            identity,
            bridge,
            runtime,
        });
        Ok(())
    }

    fn selected_tool_sources(
        &self,
        saved: &AgentChatPromptSaved,
        workspace_id: &str,
    ) -> Result<Vec<ToolSourceRecord>, String> {
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
        Ok(selected_sources)
    }

    async fn retire_unless_reusable(
        &self,
        identity: &RuntimeIdentity,
    ) -> Result<RuntimeReuse, String> {
        let previous = {
            let mut active = self.active.lock().await;
            let reuse = active.as_mut().map_or(RuntimeReuse::Relaunch, |runtime| {
                if matches!(runtime.runtime.exited(), Ok(None)) {
                    identity.reuse_from(&runtime.identity)
                } else {
                    RuntimeReuse::Relaunch
                }
            });
            if reuse != RuntimeReuse::Relaunch {
                return Ok(reuse);
            }
            active.take()
        };
        if let Some(previous) = previous {
            previous
                .shut_down("could not stop previous local Claurst runtime")
                .await?;
        }
        Ok(RuntimeReuse::Relaunch)
    }

    async fn rebind_session(
        &self,
        identity: &RuntimeIdentity,
        request: crate::claurst_local_runtime::ClaurstLocalRuntimeRequest,
        workspace: &std::path::Path,
        mcp_servers: Vec<serde_json::Value>,
    ) -> Result<bool, String> {
        let Some(mut active) = self.active.lock().await.take() else {
            return Ok(false);
        };
        let model_id = identity.model.model_id.clone();
        let readiness = self.readiness.clone();
        let workspace = workspace.to_path_buf();
        drop(active.bridge);
        let rebound = tokio::task::spawn_blocking(move || {
            ClaurstStandaloneOwner::new(
                readiness,
                SystemPrivateSettingsStore,
                SystemClaurstStandaloneLauncher,
                HttpLlamaServerReadiness::default(),
            )
            .rebind_session(
                &mut active.runtime,
                &model_id,
                request,
                &workspace,
                mcp_servers,
            )
            .map(|()| active.runtime)
        })
        .await
        .map_err(|_| "local Claurst session worker stopped unexpectedly".to_owned())?;
        let Ok(runtime) = rebound else {
            return Ok(false);
        };
        *self.active.lock().await = Some(ActiveRuntime {
            identity: identity.clone(),
            bridge: ClaurstBridgeHandle::new(runtime.bridge()),
            runtime,
        });
        Ok(true)
    }
}

pub(super) fn conversation_servers(
    config: Option<&StandaloneMcpConfig>,
    sources: &[ToolSourceRecord],
    conversation_id: &str,
    all: fn(&StandaloneMcpConfig) -> Result<Vec<serde_json::Value>, String>,
    selected: fn(
        &StandaloneMcpConfig,
        &[ToolSourceRecord],
    ) -> Result<Vec<serde_json::Value>, String>,
) -> Result<Vec<serde_json::Value>, String> {
    let entries = config
        .map(|config| {
            if sources.is_empty() {
                all(config)
            } else {
                selected(config, sources)
            }
        })
        .transpose()?
        .unwrap_or_default();
    Ok(crate::conversation_scoped_mcp::scope_named_entries(
        entries,
        conversation_id,
    ))
}
