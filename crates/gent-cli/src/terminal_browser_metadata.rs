use std::{collections::BTreeMap, path::PathBuf};

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AUTOMATIONS_CAPABILITY, CONVERSATION_CONTENT_CAPABILITY,
    FORGE_CONNECTORS_CAPABILITY,
};
use gent_types::ConversationListItem;

use crate::{chat_cli, terminal};

pub(super) async fn initial_metadata(
    index: &[ConversationListItem],
    capabilities: &[String],
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> BTreeMap<String, terminal::ConversationMetadata> {
    if !capabilities
        .iter()
        .any(|value| value == AGENT_CHAT_CONVERSATIONS_CAPABILITY)
    {
        return BTreeMap::new();
    }
    let mut metadata = BTreeMap::new();
    for item in index {
        if let Ok(summary) =
            chat_cli::summary(data_dir.clone(), no_autostart, item.conversation_id.clone()).await
        {
            let catalog = catalog(
                data_dir.clone(),
                no_autostart,
                summary.workspace_id.as_deref(),
                capabilities,
            )
            .await;
            let permission_mode = match summary.workspace_id.as_deref() {
                Some(workspace_id) => crate::permissions_cli::current_for(
                    data_dir.clone(),
                    no_autostart,
                    workspace_id.into(),
                )
                .await
                .ok()
                .flatten()
                .map_or(gent_types::PermissionMode::Default, |policy| policy.mode),
                None => gent_types::PermissionMode::Default,
            };
            let preview = if capabilities
                .iter()
                .any(|value| value == CONVERSATION_CONTENT_CAPABILITY)
            {
                crate::conversation_content::request(
                    data_dir.clone(),
                    no_autostart,
                    item.conversation_id.clone(),
                    None,
                    1,
                )
                .await
                .ok()
                .and_then(|page| page.entries.into_iter().next())
                .map(|entry| entry.text)
            } else {
                None
            };
            metadata.insert(
                item.conversation_id.clone(),
                terminal::ConversationMetadata {
                    permission_mode,
                    title: summary.title,
                    recap: summary.recap,
                    preview,
                    workspace_id: summary.workspace_id,
                    workspace_path: summary.workspace_path,
                    mcp_server_count: summary.mcp_server_count,
                    mcp_server_names: summary.mcp_server_names,
                    automation_count: count(&catalog.automation_names),
                    automation_names: catalog.automation_names,
                    automations: catalog.automations,
                    automation_runs: catalog.automation_runs,
                    forge_count: count(&catalog.forge_names),
                    forge_names: catalog.forge_names,
                    changed_file_count: summary.changed_file_count,
                    git_branch: summary.git_branch,
                },
            );
        }
    }
    metadata
}

#[derive(Default)]
pub(super) struct WorkspaceCatalog {
    pub(super) automation_names: Vec<String>,
    pub(super) automations: Vec<gent_types::AutomationDefinition>,
    pub(super) automation_runs: Vec<gent_types::AutomationRunSummary>,
    pub(super) forge_names: Vec<String>,
}

pub(super) async fn catalog(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    workspace_id: Option<&str>,
    capabilities: &[String],
) -> WorkspaceCatalog {
    let Some(workspace_id) = workspace_id else {
        return WorkspaceCatalog::default();
    };
    let automations = if capabilities
        .iter()
        .any(|value| value == AUTOMATIONS_CAPABILITY)
    {
        crate::automation_cli::list(data_dir.clone(), no_autostart, workspace_id.into())
            .await
            .map_or_else(|_| Vec::new(), |items| items)
    } else {
        Vec::new()
    };
    let forge_names = if capabilities
        .iter()
        .any(|value| value == FORGE_CONNECTORS_CAPABILITY)
    {
        crate::forge_cli::list(data_dir, no_autostart, workspace_id.into())
            .await
            .map_or_else(
                |_| Vec::new(),
                |items| items.into_iter().map(|item| item.name).collect(),
            )
    } else {
        Vec::new()
    };
    WorkspaceCatalog {
        automation_names: automations.iter().map(|item| item.name.clone()).collect(),
        automations: automations.clone(),
        automation_runs: automations
            .into_iter()
            .filter_map(|item| item.last_run)
            .collect(),
        forge_names,
    }
}

pub(super) async fn catalog_for_detail(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    detail: Option<&gent_types::AgentChatConversationDetail>,
    capabilities: &[String],
) -> WorkspaceCatalog {
    catalog(
        data_dir,
        no_autostart,
        detail.and_then(|value| value.summary.workspace_id.as_deref()),
        capabilities,
    )
    .await
}

pub(super) fn count<T>(items: &[T]) -> u16 {
    items.len().try_into().unwrap_or(u16::MAX)
}
