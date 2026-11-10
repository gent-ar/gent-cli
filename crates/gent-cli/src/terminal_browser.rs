use crate::{conversation_index, local_ipc, terminal};
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
use metadata::initial_metadata;
use provider::{model_state, pending_permission};
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
    let initial::Startup {
        state,
        capabilities,
    } = Box::pin(initial::load(data_dir.clone(), no_autostart, browse_only)).await?;
    let runtime = tokio::runtime::Handle::current();
    let request_runtime = runtime.clone();
    let request_data_dir = data_dir.clone();
    let view_runtime = runtime.clone();
    let view_data_dir = data_dir.clone();
    let view_capabilities = capabilities.clone();
    let render_template_runtime = runtime.clone();
    let render_template_data_dir = data_dir.clone();
    let documents_runtime = runtime.clone();
    let documents_data_dir = data_dir.clone();
    let list_templates_runtime = runtime.clone();
    let list_templates_data_dir = data_dir.clone();
    let sessions_runtime = runtime.clone();
    let sessions_data_dir = data_dir.clone();
    let login_runtime = runtime.clone();
    let login_data_dir = data_dir.clone();
    let preference_dir = data_dir.clone().unwrap_or_else(local_ipc::default_data_dir);
    let show_thinking = crate::terminal_preferences::load(&preference_dir).unwrap_or(false);
    let state = state.with_show_thinking(show_thinking);
    let conversations_runtime = runtime.clone();
    let conversations_data_dir = data_dir.clone();
    let conversations_capabilities = capabilities.clone();
    let ports = terminal::TerminalPorts {
        request: Box::new(move |intent| {
            submit::request(
                &request_runtime,
                request_data_dir.clone(),
                no_autostart,
                intent,
            )
        }),
        refresh: Box::new(move |conversation_id| {
            tokio::task::block_in_place(|| {
                view_runtime.block_on(read_view(
                    view_data_dir.clone(),
                    no_autostart,
                    conversation_id,
                    &view_capabilities,
                ))
            })
        }),
        render_template: Box::new(move |template_id, variables| {
            tokio::task::block_in_place(|| {
                render_template_runtime.block_on(crate::prompt_templates_cli::render(
                    render_template_data_dir.clone(),
                    no_autostart,
                    template_id,
                    variables,
                ))
            })
            .map_err(|error| error.to_string())
        }),
        list_documents: Box::new(move |workspace_id| {
            tokio::task::block_in_place(|| {
                documents_runtime.block_on(crate::workspace_documents_cli::list(
                    documents_data_dir.clone(),
                    no_autostart,
                    workspace_id,
                ))
            })
            .map_err(|error| error.to_string())
        }),
        list_templates: Box::new(move || {
            tokio::task::block_in_place(|| {
                list_templates_runtime.block_on(crate::prompt_templates_cli::list(
                    list_templates_data_dir.clone(),
                    no_autostart,
                ))
            })
            .map_err(|error| error.to_string())
        }),
        create_session: Box::new(move |session| {
            tokio::task::block_in_place(|| {
                sessions_runtime.block_on(crate::session_cli::create(
                    sessions_data_dir.clone(),
                    no_autostart,
                    session,
                ))
            })
            .map_err(|error| error.to_string())
        }),
        login: Box::new(move |provider| {
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
        }),
        save_thinking: Box::new(move |show_thinking| {
            crate::terminal_preferences::save(&preference_dir, show_thinking)
        }),
        workspace_git: workspace_git_port(runtime, data_dir, no_autostart),
        conversations: Box::new(move || {
            tokio::task::block_in_place(|| {
                conversations_runtime.block_on(async {
                    let index =
                        conversation_index::request(conversations_data_dir.clone(), no_autostart)
                            .await
                            .map_err(|error| error.to_string())?;
                    let metadata = initial_metadata(
                        &index,
                        &conversations_capabilities,
                        conversations_data_dir.clone(),
                        no_autostart,
                        true,
                    )
                    .await;
                    Ok((index, metadata))
                })
            })
        }),
    };
    if browse_only {
        terminal::run(state, ports)?;
    } else {
        terminal::run_inline(state, ports)?;
    }
    Ok(())
}

fn workspace_git_port<'a>(
    runtime: tokio::runtime::Handle,
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Box<dyn FnMut(Option<String>) -> Option<terminal::WorkspaceGitResult> + 'a> {
    let mut pending: Option<std::sync::mpsc::Receiver<terminal::WorkspaceGitResult>> = None;
    Box::new(move |request| {
        if let Some(workspace_id) = request {
            if pending.is_none() {
                let (sender, receiver) = std::sync::mpsc::channel();
                let runtime = runtime.clone();
                let data_dir = data_dir.clone();
                std::thread::spawn(move || {
                    let result = runtime
                        .block_on(crate::workspace_git_cli::status(
                            data_dir,
                            no_autostart,
                            workspace_id.clone(),
                        ))
                        .map_err(|error| error.to_string());
                    let _ = sender.send((workspace_id, result));
                });
                pending = Some(receiver);
            }
            return None;
        }
        let result = pending.as_ref()?.try_recv().ok()?;
        pending = None;
        Some(result)
    })
}
#[cfg(test)]
#[path = "terminal_browser_tests.rs"]
mod tests;
