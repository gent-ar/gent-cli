use std::path::PathBuf;

use crate::terminal::{self, UiRequest};

use crate::terminal_browser::automation;

#[path = "terminal_browser_command.rs"]
mod command;
#[path = "terminal_browser_submit_requests.rs"]
mod requests;

pub(super) fn request(
    runtime: &tokio::runtime::Handle,
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: terminal::UiRequest,
) -> Result<terminal::UiRequestResult, String> {
    tokio::task::block_in_place(|| runtime.block_on(resolve(data_dir, no_autostart, request)))
}

async fn resolve(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: terminal::UiRequest,
) -> Result<terminal::UiRequestResult, String> {
    let queued = matches!(request, UiRequest::Queue { .. });
    match request {
        UiRequest::Create {
            selection,
            session_id,
        } => requests::create(data_dir, no_autostart, selection, session_id).await,
        UiRequest::Send {
            conversation_id,
            text,
            attachments,
        }
        | UiRequest::Queue {
            conversation_id,
            text,
            attachments,
        } => {
            requests::send(
                data_dir,
                no_autostart,
                conversation_id,
                text,
                attachments,
                queued,
            )
            .await
        }
        UiRequest::RunAutomation {
            automation_id,
            conversation_id,
        } => automation::run(data_dir, no_autostart, automation_id, conversation_id).await,
        UiRequest::InvokeCommand {
            conversation_id,
            name,
            arguments,
            session_id,
        } => {
            command::invoke(
                data_dir,
                no_autostart,
                conversation_id,
                name,
                arguments,
                session_id,
            )
            .await
        }
        UiRequest::Switch {
            conversation_id,
            parent_run_id,
            selection,
            context_policy,
        } => {
            requests::switch(
                data_dir,
                no_autostart,
                conversation_id,
                parent_run_id,
                selection,
                context_policy,
            )
            .await
        }
        UiRequest::Permission { response } => {
            requests::permission(data_dir, no_autostart, response).await
        }
        UiRequest::SetPermissionMode {
            conversation_id,
            workspace_id,
            mode,
            bypass_consent,
        } => {
            requests::set_permission_mode(
                data_dir,
                no_autostart,
                conversation_id,
                workspace_id,
                mode,
                bypass_consent,
            )
            .await
        }
        UiRequest::SteerQueued {
            conversation_id,
            message_ids,
        } => requests::steer_queued(data_dir, no_autostart, conversation_id, message_ids).await,
        UiRequest::CancelQueued {
            conversation_id,
            message_id,
        } => requests::cancel_queued(data_dir, no_autostart, conversation_id, message_id).await,
        UiRequest::ContinueFromHistory {
            conversation_id,
            message_id,
        } => {
            requests::continue_from_history(data_dir, no_autostart, conversation_id, message_id)
                .await
        }
        UiRequest::Interrupt {
            conversation_id,
            run_id,
        } => requests::interrupt(data_dir, no_autostart, conversation_id, run_id).await,
    }
}
