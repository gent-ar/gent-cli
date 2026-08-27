use crate::{
    chat_cli, conversation_activity, conversation_index, conversation_status,
    conversation_timeline, local_ipc, terminal,
};
use gent_protocol::{
    AGENT_CHAT_CONVERSATIONS_CAPABILITY, AGENT_CHAT_INTENTS_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_CAPABILITY, CONVERSATION_ACTIVITY_CAPABILITY,
    CONVERSATION_STATUS_CAPABILITY, CONVERSATION_TIMELINE_CAPABILITY,
};
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection};
use std::path::PathBuf;
#[path = "terminal_browser_automation.rs"]
pub(crate) mod automation;
#[cfg(test)]
pub(crate) use result::delivery_notice;
#[path = "terminal_browser_initial.rs"]
mod initial;
#[path = "terminal_browser_metadata.rs"]
mod metadata;
#[path = "terminal_browser_provider.rs"]
mod provider;
#[path = "terminal_browser_result.rs"]
pub(crate) mod result;
use initial::initial_view;
use metadata::initial_metadata;
use provider::{local_model_ids, model_state, pending_permission};
#[path = "terminal_browser_submit.rs"]
mod submit;
pub(crate) async fn open(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    terminal::require_interactive()?;
    let mut index = conversation_index::request(data_dir.clone(), no_autostart).await?;
    let (_, capabilities) =
        local_ipc::connect_and_negotiate(data_dir.clone(), no_autostart).await?;
    let enabled = capabilities
        .0
        .iter()
        .any(|value| value == AGENT_CHAT_INTENTS_CAPABILITY);
    if enabled && index.is_empty() {
        chat_cli::create(
            data_dir.clone(),
            no_autostart,
            AgentChatSelection {
                provider: AgentChatProvider::Claurst,
                model: gent_protocol::DEFAULT_LOCAL_MODEL_ID.into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
            None,
        )
        .await?;
        index = conversation_index::request(data_dir.clone(), no_autostart).await?;
    }
    let view = initial_view(&index, &capabilities.0, data_dir.clone(), no_autostart).await;
    let metadata = initial_metadata(&index, &capabilities.0, data_dir.clone(), no_autostart).await;
    let local_model_ids = local_model_ids(data_dir.clone(), no_autostart, &capabilities.0).await;
    let mut sessions = Vec::new();
    let workspaces = metadata
        .values()
        .filter_map(|item| item.workspace_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for workspace_id in workspaces {
        if let Ok(items) =
            crate::session_cli::list(data_dir.clone(), no_autostart, workspace_id).await
        {
            sessions.extend(items);
        }
    }
    let runtime = tokio::runtime::Handle::current();
    let request_runtime = runtime.clone();
    let request_data_dir = data_dir.clone();
    let view_runtime = runtime.clone();
    let view_data_dir = data_dir.clone();
    let view_capabilities = capabilities.0.clone();
    let template_runtime = runtime.clone();
    let template_data_dir = data_dir.clone();
    let documents_runtime = runtime.clone();
    let documents_data_dir = data_dir.clone();
    let templates_runtime = runtime.clone();
    let templates_data_dir = data_dir.clone();
    let sessions_runtime = runtime.clone();
    let sessions_data_dir = data_dir.clone();
    let login_data_dir = data_dir.clone();
    let preference_dir = data_dir.clone().unwrap_or_else(local_ipc::default_data_dir);
    let show_thinking = crate::terminal_preferences::load(&preference_dir).unwrap_or(false);
    terminal::run(
        terminal::UiState::new(index)
            .with_chat_input(enabled)
            .with_metadata(metadata)
            .with_sessions(sessions)
            .with_view(view)
            .with_local_model_ids(local_model_ids)
            .with_show_thinking(show_thinking),
        move |intent| {
            submit::request(
                &request_runtime,
                request_data_dir.clone(),
                no_autostart,
                intent,
            )
        },
        move |conversation_id| {
            tokio::task::block_in_place(|| {
                view_runtime.block_on(read_view(
                    view_data_dir.clone(),
                    no_autostart,
                    conversation_id,
                    &view_capabilities,
                ))
            })
        },
        move |template_id, variables| {
            tokio::task::block_in_place(|| {
                template_runtime.block_on(crate::prompt_templates_cli::render(
                    template_data_dir.clone(),
                    no_autostart,
                    template_id,
                    variables,
                ))
            })
            .map_err(|error| error.to_string())
        },
        move |workspace_id| {
            tokio::task::block_in_place(|| {
                documents_runtime.block_on(crate::workspace_documents_cli::list(
                    documents_data_dir.clone(),
                    no_autostart,
                    workspace_id,
                ))
            })
            .map_err(|error| error.to_string())
        },
        move || {
            tokio::task::block_in_place(|| {
                templates_runtime.block_on(crate::prompt_templates_cli::list(
                    templates_data_dir.clone(),
                    no_autostart,
                ))
            })
            .map_err(|error| error.to_string())
        },
        move |session| {
            tokio::task::block_in_place(|| {
                sessions_runtime.block_on(crate::session_cli::create(
                    sessions_data_dir.clone(),
                    no_autostart,
                    session,
                ))
            })
            .map_err(|error| error.to_string())
        },
        move |provider| {
            let provider = match provider {
                AgentChatProvider::Claude => crate::provider_auth_cli::ProviderArgument::Claude,
                AgentChatProvider::Codex => crate::provider_auth_cli::ProviderArgument::Codex,
                AgentChatProvider::Claurst => {
                    return Err("Gent uses local models and does not require a login.".into());
                }
            };
            crate::provider_auth_cli::login_interactive(login_data_dir.clone(), provider)
        },
        move |show_thinking| crate::terminal_preferences::save(&preference_dir, show_thinking),
    )?;
    Ok(())
}
async fn read_view(
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
    let preview = transcript
        .as_ref()
        .and_then(|page| latest_preview(&page.events));
    Ok(
        terminal::ConversationView::new(&conversation_id, status, transcript)
            .with_current_run_id(detail.as_ref().map(|value| value.current_run_id.clone()))
            .with_selection(detail.as_ref().map(|value| value.summary.selection.clone()))
            .with_metadata(
                detail
                    .as_ref()
                    .and_then(|value| value.summary.title.clone()),
                detail
                    .as_ref()
                    .and_then(|value| value.summary.recap.clone()),
                preview,
                detail
                    .as_ref()
                    .and_then(|value| value.summary.workspace_id.clone()),
                detail
                    .as_ref()
                    .and_then(|value| value.summary.workspace_path.clone()),
                detail
                    .as_ref()
                    .map_or(0, |value| value.summary.mcp_server_count),
                detail
                    .as_ref()
                    .map_or_else(Vec::new, |value| value.summary.mcp_server_names.clone()),
                metadata::count(&catalog.automation_names),
                catalog.automation_names,
                catalog.automations,
                catalog.automation_runs,
                metadata::count(&catalog.forge_names),
                catalog.forge_names,
                detail
                    .as_ref()
                    .and_then(|value| value.summary.changed_file_count),
                detail
                    .as_ref()
                    .and_then(|value| value.summary.git_branch.clone()),
            )
            .with_activity(activity)
            .with_timeline(timeline)
            .with_local_model_state(model)
            .with_pending_permission(pending_permission),
    )
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
#[path = "terminal_browser_tests.rs"]
mod tests;
