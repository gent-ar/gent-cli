use std::path::PathBuf;

use gent_ports::{AgentChatConversationConfigLedger, AgentChatWorkspaceLedger, ToolSourceLedger};
use gent_runtime::AgentChatReadService;
use gent_types::ToolSourceRecord;

use super::{ActiveRuntime, StandaloneClaurstRuntimeFactory};
use crate::standalone_claurst_runtime_identity::RuntimeIdentity;
use crate::standalone_mcp_config::StandaloneMcpConfig;
use crate::{
    claurst_acp_bridge::ClaurstBridgeHandle,
    claurst_local_runtime_owner::{HttpLlamaServerReadiness, SystemPrivateSettingsStore},
    claurst_standalone_owner::ClaurstStandaloneOwner,
};

struct Session<'a> {
    conversation_id: String,
    run_id: &'a str,
    identity: RuntimeIdentity,
}

impl StandaloneClaurstRuntimeFactory {
    pub(super) async fn start_selected(
        &self,
        conversation_id: &str,
        run_id: &str,
        tool_source_ids: &[String],
        busy: &[String],
    ) -> Result<gent_types::PolicyRecord, String> {
        let selection = AgentChatReadService::new(self.ledger.clone())
            .run_selection(conversation_id, run_id)
            .map_err(|error| error.to_string())?;
        let workspace_record = self
            .ledger
            .agent_chat_workspace_for_run(conversation_id, run_id)
            .map_err(|error| error.to_string())?;
        let workspace_id = workspace_record.workspace_id.clone();
        let workspace = PathBuf::from(workspace_record.canonical_path);
        let permission = crate::permission_workspace::policy_for_conversation(
            &self.ledger,
            conversation_id,
            &workspace_id,
        )
        .map_err(|error| error.to_string())?;
        crate::permission_workspace::require_supported_launch(
            &permission,
            gent_types::AgentChatProvider::Claurst,
        )
        .map_err(|error| error.to_string())?;
        if !workspace.is_absolute() {
            return Err("the selected Gent workspace is not absolute".into());
        }

        let config = self.config.as_ref().ok_or_else(|| {
            "Claurst is selected but its local Claurst and llama.cpp executables are not installed"
                .to_owned()
        })?;
        let selected_sources = self.selected_tool_sources(tool_source_ids, &workspace_id)?;
        let conversation_config = self
            .ledger
            .current_conversation_config(conversation_id)
            .map_err(|error| error.to_string())?;
        let disallowed_tools = conversation_config
            .as_ref()
            .map(|config| config.disallowed_tools.as_slice())
            .unwrap_or_default();
        let mcp_config_digest = config
            .mcp_config
            .as_ref()
            .map(crate::standalone_mcp_config::StandaloneMcpConfig::digest)
            .transpose()?
            .map(|digest| {
                format!(
                    "{digest}:{}",
                    conversation_config
                        .as_ref()
                        .map_or(0, |config| config.revision)
                )
            });
        let permission_mode = permission.mode;
        let conversation_id = conversation_id.to_owned();
        let mut identity = RuntimeIdentity::new(
            &selection,
            conversation_id.clone(),
            workspace.clone(),
            permission_mode,
            mcp_config_digest,
            tool_source_ids,
        );
        let binding = if permission.requires_sandbox() {
            let epoch = gent_ports::Ledger::host_ingress(&self.ledger)
                .map_err(|error| error.to_string())?
                .epoch;
            Some(
                gent_types::SandboxPermissionBinding::new(
                    conversation_id.clone(),
                    run_id.to_owned(),
                    permission.clone(),
                    epoch,
                )
                .map_err(|_| "invalid Claurst permission authority")?,
            )
        } else {
            None
        };
        if let Some(binding) = &binding {
            identity = identity.with_permission_binding(binding.clone());
        }
        if self.reuse_ready(&conversation_id, run_id, &identity).await {
            return Ok(permission);
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
            disallowed_tools,
        )?;
        let launch_mcp_servers = conversation_servers(
            config.mcp_config.as_ref(),
            &selected_sources,
            &conversation_id,
            StandaloneMcpConfig::claurst_servers,
            StandaloneMcpConfig::selected_claurst_servers,
            disallowed_tools,
        )?;
        self.launch_session(
            Session {
                conversation_id,
                run_id,
                identity,
            },
            request,
            workspace,
            launch_mcp_servers,
            busy,
        )
        .await
        .map(|()| permission)
    }

    fn selected_tool_sources(
        &self,
        tool_source_ids: &[String],
        workspace_id: &str,
    ) -> Result<Vec<ToolSourceRecord>, String> {
        let selected_sources = tool_source_ids
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
    disallowed_tools: &[String],
) -> Result<Vec<serde_json::Value>, String> {
    let mut entries = config
        .map(|config| {
            if sources.is_empty() {
                all(config)
            } else {
                selected(config, sources)
            }
        })
        .transpose()?
        .unwrap_or_default();
    entries.retain(|entry| {
        !crate::conversation_scoped_mcp::server_is_disallowed(
            entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            disallowed_tools,
        )
    });
    Ok(crate::conversation_scoped_mcp::scope_named_entries(
        entries,
        conversation_id,
    ))
}

#[path = "standalone_claurst_runtime_factory_start.rs"]
mod start;
