use std::{collections::BTreeMap, path::PathBuf};

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AUTOMATIONS_CAPABILITY, FORGE_CONNECTORS_CAPABILITY,
};
use gent_types::ConversationListItem;

use crate::{chat_cli, terminal};

pub(super) async fn initial_metadata(
    index: &[ConversationListItem],
    capabilities: &[String],
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    with_status: bool,
) -> BTreeMap<String, terminal::ConversationMetadata> {
    if !capabilities
        .iter()
        .any(|value| value == AGENT_CHAT_CONVERSATIONS_CAPABILITY)
    {
        return BTreeMap::new();
    }
    let mut tasks = tokio::task::JoinSet::new();
    for item in index {
        let data_dir = data_dir.clone();
        let conversation_id = item.conversation_id.clone();
        tasks.spawn(async move {
            let summary =
                chat_cli::summary(data_dir.clone(), no_autostart, conversation_id.clone())
                    .await
                    .ok()?;
            let running = with_status
                && crate::conversation_status::request(data_dir, no_autostart, conversation_id)
                    .await
                    .is_ok_and(|status| status.runs.iter().any(|run| run.active_turn_id.is_some()));
            Some((summary, running))
        });
    }
    let mut metadata = BTreeMap::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(Some((summary, running))) = result {
            metadata.insert(
                summary.conversation_id.clone(),
                terminal::ConversationMetadata {
                    title: summary.title,
                    preview: summary.recap.clone(),
                    recap: summary.recap,
                    workspace_id: summary.workspace_id,
                    workspace_path: summary.workspace_path,
                    mcp_server_count: summary.mcp_server_count,
                    mcp_server_names: summary.mcp_server_names,
                    updated_at_unix_ms: summary.updated_at_unix_ms,
                    running,
                    ..terminal::ConversationMetadata::default()
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
            .unwrap_or_default()
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
