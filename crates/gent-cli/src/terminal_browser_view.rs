use std::path::PathBuf;

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
    CONVERSATION_ACTIVITY_CAPABILITY, CONVERSATION_STATUS_CAPABILITY,
    CONVERSATION_TIMELINE_CAPABILITY,
    agent_chat_commands::{CommandCatalog, CommandListing},
};

use super::{metadata, model_state, pending_permission};
use crate::{
    chat_cli, conversation_activity, conversation_status, conversation_timeline, prompt_hold,
    terminal,
};

pub(super) async fn read_view(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    caps: &[String],
) -> Result<terminal::ConversationView, String> {
    let has = |capability: &str| caps.iter().any(|value| value == capability);
    let text = |error: Box<dyn std::error::Error>| error.to_string();
    let id = || conversation_id.clone();
    let (status, transcript, detail, timeline) = tokio::join!(
        async {
            if has(CONVERSATION_STATUS_CAPABILITY) {
                conversation_status::request(data_dir.clone(), no_autostart, id())
                    .await
                    .map(Some)
                    .map_err(text)
            } else {
                Ok(None)
            }
        },
        async {
            if has(AGENT_CHAT_TRANSCRIPT_CAPABILITY) {
                chat_cli::transcript_all(data_dir.clone(), no_autostart, id())
                    .await
                    .map(Some)
                    .map_err(text)
            } else {
                Ok(None)
            }
        },
        async {
            if has(AGENT_CHAT_CONVERSATIONS_CAPABILITY) {
                chat_cli::detail(data_dir.clone(), no_autostart, id())
                    .await
                    .map(Some)
                    .map_err(text)
            } else {
                Ok(None)
            }
        },
        async {
            if has(CONVERSATION_TIMELINE_CAPABILITY) {
                conversation_timeline::request(data_dir.clone(), no_autostart, id())
                    .await
                    .map(Some)
                    .map_err(text)
            } else {
                Ok(None)
            }
        },
    );
    let (status, transcript, detail, timeline) = (status?, transcript?, detail?, timeline?);
    let workspace_id = detail
        .as_ref()
        .and_then(|value| value.summary.workspace_id.clone());
    let (activity, catalog, model, pending_permission, permission_mode, commands) = tokio::join!(
        async {
            if !has(CONVERSATION_ACTIVITY_CAPABILITY) {
                return Ok(None);
            }
            let mut facts = Vec::new();
            for run in status.iter().flat_map(|status| &status.runs) {
                facts.extend(
                    conversation_activity::all(
                        data_dir.clone(),
                        no_autostart,
                        id(),
                        run.run_id.clone(),
                    )
                    .await
                    .map_err(text)?,
                );
            }
            Ok::<_, String>(Some(facts))
        },
        metadata::catalog_for_detail(data_dir.clone(), no_autostart, detail.as_ref(), caps),
        model_state(data_dir.clone(), no_autostart, detail.as_ref(), caps),
        pending_permission(data_dir.clone(), no_autostart, &conversation_id, caps),
        async {
            match workspace_id {
                Some(workspace_id) => crate::permissions_cli::current_for(
                    data_dir.clone(),
                    no_autostart,
                    workspace_id,
                )
                .await
                .ok()
                .flatten()
                .map(|policy| policy.mode),
                None => None,
            }
        },
        command_catalog(
            data_dir.clone(),
            no_autostart,
            &conversation_id,
            detail.as_ref()
        ),
    );
    let (activity, model, pending_permission) = (activity?, model?, pending_permission?);
    let install_hold = match (activity.as_deref(), detail.as_ref()) {
        (Some(facts), Some(detail)) => {
            match prompt_hold::current_hold(facts, &detail.current_run_id) {
                Some(hold) => prompt_hold::install_review(
                    data_dir.clone(),
                    no_autostart,
                    &conversation_id,
                    &detail.current_run_id,
                    &hold,
                )
                .await
                .ok()
                .flatten(),
                None => None,
            }
        }
        _ => None,
    };
    let preview = transcript
        .as_ref()
        .and_then(|page| latest_preview(&page.events));
    let mut metadata = conversation_metadata(detail.as_ref(), preview, catalog);
    metadata.permission_mode = permission_mode.unwrap_or_default();
    Ok(
        terminal::ConversationView::new(&conversation_id, status, transcript)
            .with_current_run_id(detail.as_ref().map(|value| value.current_run_id.clone()))
            .with_selection(detail.as_ref().map(|value| value.summary.selection.clone()))
            .with_metadata(metadata)
            .with_activity(activity)
            .with_timeline(timeline)
            .with_local_model_state(model)
            .with_pending_permission(pending_permission)
            .with_install_hold(install_hold)
            .with_commands(commands),
    )
}

type CommandScope = (
    String,
    Option<gent_types::AgentChatProvider>,
    Option<String>,
);

static COMMANDS: std::sync::Mutex<Option<(CommandScope, CommandCatalog)>> =
    std::sync::Mutex::new(None);

async fn command_catalog(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &str,
    detail: Option<&gent_types::AgentChatConversationDetail>,
) -> Option<CommandCatalog> {
    let scope = (
        conversation_id.to_owned(),
        detail.map(|value| value.summary.selection.provider),
        detail.and_then(|value| value.summary.workspace_path.clone()),
    );
    if let Some(catalog) = remembered_for(&scope) {
        return Some(catalog);
    }
    let catalog =
        crate::command_catalog_cli::read(data_dir, no_autostart, Some(conversation_id.to_owned()))
            .await
            .ok()
            .flatten()?;
    remember(scope, catalog.clone());
    Some(catalog)
}

fn remember(scope: CommandScope, catalog: CommandCatalog) {
    *COMMANDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((scope, catalog));
}

pub(super) fn after_login<T, E>(result: Result<T, E>) -> Result<T, E> {
    if result.is_ok() {
        *COMMANDS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
    result
}

fn remembered_for(scope: &CommandScope) -> Option<CommandCatalog> {
    let remembered = COMMANDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    settled_for(remembered, scope)
}

fn settled_for(
    remembered: Option<(CommandScope, CommandCatalog)>,
    scope: &CommandScope,
) -> Option<CommandCatalog> {
    remembered
        .filter(|(remembered, catalog)| {
            remembered == scope && catalog.listing != CommandListing::Loading
        })
        .map(|(_, catalog)| catalog)
}

fn conversation_metadata(
    detail: Option<&gent_types::AgentChatConversationDetail>,
    preview: Option<String>,
    catalog: metadata::WorkspaceCatalog,
) -> terminal::ConversationMetadata {
    terminal::ConversationMetadata {
        title: detail.and_then(|value| value.summary.title.clone()),
        recap: detail.and_then(|value| value.summary.recap.clone()),
        preview,
        workspace_id: detail.and_then(|value| value.summary.workspace_id.clone()),
        workspace_path: detail.and_then(|value| value.summary.workspace_path.clone()),
        mcp_server_count: detail.map_or(0, |value| value.summary.mcp_server_count),
        mcp_server_names: detail
            .map_or_else(Vec::new, |value| value.summary.mcp_server_names.clone()),
        automation_count: metadata::count(&catalog.automation_names),
        automation_names: catalog.automation_names,
        automations: catalog.automations,
        automation_runs: catalog.automation_runs,
        forge_count: metadata::count(&catalog.forge_names),
        forge_names: catalog.forge_names,
        updated_at_unix_ms: detail.map_or(0, |value| value.summary.updated_at_unix_ms),
        ..terminal::ConversationMetadata::default()
    }
}

fn latest_preview(events: &[gent_types::NormalizedTranscriptEvent]) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| {
            !event.is_partial
                && matches!(
                    event.kind,
                    gent_types::NormalizedTranscriptKind::AssistantMessage
                        | gent_types::NormalizedTranscriptKind::UserMessage
                )
        })
        .map(|event| event.text.clone())
}

#[cfg(test)]
#[path = "terminal_browser_view_tests.rs"]
mod tests;
