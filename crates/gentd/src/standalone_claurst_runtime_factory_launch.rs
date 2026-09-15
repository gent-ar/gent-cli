use std::path::PathBuf;

use gent_ports::{AgentChatWorkspaceLedger, ToolSourceLedger};
use gent_runtime::AgentChatReadService;
use gent_types::{
    AgentChatEffort, AgentChatMode, AgentChatPromptSaved, AgentChatSelection, PermissionMode,
    ToolSourceRecord,
};

use super::{ActiveRuntime, StandaloneClaurstRuntimeFactory};
use crate::{
    claurst_acp_bridge::ClaurstBridgeHandle,
    claurst_local_runtime_owner::{
        HttpLlamaServerReadiness, SystemClaurstStandaloneLauncher, SystemPrivateSettingsStore,
    },
    claurst_standalone_owner::ClaurstStandaloneOwner,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RuntimeIdentity {
    pub(super) model_id: String,
    mode: AgentChatMode,
    effort: AgentChatEffort,
    workspace: PathBuf,
    permission_mode: PermissionMode,
    mcp_config_digest: Option<String>,
    tool_source_ids: Vec<String>,
}

impl RuntimeIdentity {
    pub(super) fn new(
        selection: &AgentChatSelection,
        workspace: PathBuf,
        permission_mode: PermissionMode,
        mcp_config_digest: Option<String>,
        tool_source_ids: &[String],
    ) -> Self {
        let mut tool_source_ids = tool_source_ids.to_vec();
        tool_source_ids.sort();
        Self {
            model_id: selection.model.clone(),
            mode: selection.mode,
            effort: selection.effort,
            workspace,
            permission_mode,
            mcp_config_digest,
            tool_source_ids,
        }
    }
}

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
        let selected = !selected_sources.is_empty();
        let mcp_config_digest = config
            .mcp_config
            .as_ref()
            .map(crate::standalone_mcp_config::StandaloneMcpConfig::digest)
            .transpose()?;
        let permission_mode = permission.mode;
        let identity = RuntimeIdentity::new(
            &selection,
            workspace.clone(),
            permission_mode,
            mcp_config_digest,
            &saved.tool_source_ids,
        );
        if self.retire_unless_reusable(&identity).await? {
            return Ok(());
        }

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
        let launch_model_id = identity.model_id.clone();
        let launch_workspace = workspace;
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

    async fn retire_unless_reusable(&self, identity: &RuntimeIdentity) -> Result<bool, String> {
        let previous = {
            let mut active = self.active.lock().await;
            let reusable = active.as_mut().is_some_and(|runtime| {
                runtime.identity == *identity && matches!(runtime.runtime.exited(), Ok(None))
            });
            if reusable {
                return Ok(true);
            }
            active.take()
        };
        if let Some(previous) = previous {
            previous
                .shut_down("could not stop previous local Claurst runtime")
                .await?;
        }
        Ok(false)
    }
}
