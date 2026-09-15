use crate::{chat_cli, conversation_index, local_ipc, terminal};
use gent_protocol::AGENT_CHAT_INTENTS_CAPABILITY;
use gent_types::AgentChatProvider;
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
use provider::{model_catalog, model_state, pending_permission};
#[path = "terminal_browser_submit.rs"]
mod submit;
#[path = "terminal_browser_view.rs"]
mod view;
use view::read_view;
pub(crate) async fn open(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    browse_only: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    terminal::require_interactive()?;
    let mut index = conversation_index::request(data_dir.clone(), no_autostart).await?;
    let (_, capabilities) =
        local_ipc::connect_and_negotiate(data_dir.clone(), no_autostart).await?;
    let enabled = !browse_only
        && capabilities
            .0
            .iter()
            .any(|value| value == AGENT_CHAT_INTENTS_CAPABILITY);
    if enabled && index.is_empty() {
        chat_cli::create(data_dir.clone(), no_autostart, None, None).await?;
        index = conversation_index::request(data_dir.clone(), no_autostart).await?;
    }
    let view = initial_view(&index, &capabilities.0, data_dir.clone(), no_autostart).await;
    let metadata = initial_metadata(&index, &capabilities.0, data_dir.clone(), no_autostart).await;
    let model_catalog = model_catalog(data_dir.clone(), no_autostart, &capabilities.0).await;
    let command_catalog = crate::command_catalog_cli::read(data_dir.clone(), no_autostart, None)
        .await
        .ok()
        .flatten();
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
    let login_runtime = runtime.clone();
    let login_data_dir = data_dir.clone();
    let preference_dir = data_dir.clone().unwrap_or_else(local_ipc::default_data_dir);
    let show_thinking = crate::terminal_preferences::load(&preference_dir).unwrap_or(false);
    terminal::run(
        terminal::UiState::new(index)
            .with_chat_input(enabled)
            .with_metadata(metadata)
            .with_sessions(sessions)
            .with_view(view)
            .with_model_catalog(model_catalog)
            .with_command_catalog(command_catalog)
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
            tokio::task::block_in_place(|| {
                login_runtime.block_on(crate::provider_auth_cli::login_interactive(
                    login_data_dir.clone(),
                    no_autostart,
                    provider,
                ))
            })
        },
        move |show_thinking| crate::terminal_preferences::save(&preference_dir, show_thinking),
    )?;
    Ok(())
}
#[cfg(test)]
#[path = "terminal_browser_tests.rs"]
mod tests;
