use std::path::PathBuf;

use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_CAPABILITY,
    CONVERSATION_ACTIVITY_CAPABILITY, CONVERSATION_STATUS_CAPABILITY,
    CONVERSATION_TIMELINE_CAPABILITY,
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
    let status = if caps
        .iter()
        .any(|value| value == CONVERSATION_STATUS_CAPABILITY)
    {
        Some(
            conversation_status::request(data_dir.clone(), no_autostart, conversation_id.clone())
                .await
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let transcript = if caps
        .iter()
        .any(|value| value == AGENT_CHAT_TRANSCRIPT_CAPABILITY)
    {
        Some(
            chat_cli::transcript_all(data_dir.clone(), no_autostart, conversation_id.clone())
                .await
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let detail = if caps
        .iter()
        .any(|value| value == AGENT_CHAT_CONVERSATIONS_CAPABILITY)
    {
        Some(
            chat_cli::detail(data_dir.clone(), no_autostart, conversation_id.clone())
                .await
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let activity = if caps
        .iter()
        .any(|value| value == CONVERSATION_ACTIVITY_CAPABILITY)
    {
        let mut facts = Vec::new();
        if let Some(status) = &status {
            for run in &status.runs {
                facts.extend(
                    conversation_activity::all(
                        data_dir.clone(),
                        no_autostart,
                        conversation_id.clone(),
                        run.run_id.clone(),
                    )
                    .await
                    .map_err(|error| error.to_string())?,
                );
            }
        }
        Some(facts)
    } else {
        None
    };
    let timeline = if caps
        .iter()
        .any(|value| value == CONVERSATION_TIMELINE_CAPABILITY)
    {
        Some(
            conversation_timeline::request(data_dir.clone(), no_autostart, conversation_id.clone())
                .await
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let catalog =
        metadata::catalog_for_detail(data_dir.clone(), no_autostart, detail.as_ref(), caps).await;
    let model = model_state(data_dir.clone(), no_autostart, detail.as_ref(), caps).await?;
    let pending_permission = pending_permission(
        data_dir.clone(),
        no_autostart,
        &conversation_id,
        detail.as_ref(),
        caps,
    )
    .await?;
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
    let commands = crate::command_catalog_cli::read(
        data_dir.clone(),
        no_autostart,
        Some(conversation_id.clone()),
    )
    .await
    .ok()
    .flatten();
    let preview = transcript
        .as_ref()
        .and_then(|page| latest_preview(&page.events));
    Ok(
        terminal::ConversationView::new(&conversation_id, status, transcript)
            .with_current_run_id(detail.as_ref().map(|value| value.current_run_id.clone()))
            .with_selection(detail.as_ref().map(|value| value.summary.selection.clone()))
            .with_metadata(conversation_metadata(detail.as_ref(), preview, catalog))
            .with_activity(activity)
            .with_timeline(timeline)
            .with_local_model_state(model)
            .with_pending_permission(pending_permission)
            .with_install_hold(install_hold)
            .with_commands(commands),
    )
}

fn conversation_metadata(
    detail: Option<&gent_types::AgentChatConversationDetail>,
    preview: Option<String>,
    catalog: metadata::WorkspaceCatalog,
) -> terminal::ConversationMetadata {
    terminal::ConversationMetadata {
        permission_mode: gent_types::PermissionMode::AskEveryTime,
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
        changed_file_count: detail.and_then(|value| value.summary.changed_file_count),
        git_branch: detail.and_then(|value| value.summary.git_branch.clone()),
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
