use std::path::PathBuf;

use crate::{chat_cli, terminal};

use crate::terminal_browser::{
    automation,
    result::{delivery_notice, result},
};

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
    match request {
        terminal::UiRequest::Create {
            selection,
            session_id,
        } => {
            let workspace = std::env::current_dir()
                .map_err(|_| "Gent could not determine the current workspace.".to_owned())?;
            let (conversation_id, run_id) =
                chat_cli::create(data_dir.clone(), no_autostart, selection, Some(workspace))
                    .await
                    .map_err(|error| error.to_string())?;
            let mut created = result(
                conversation_id.0,
                Some(run_id.0),
                if session_id.is_some() {
                    "Conversation created and attached to the selected session."
                } else {
                    "Conversation created; choose a prompt to persist."
                },
            );
            if let Some(session_id) = session_id {
                created.session = Some(
                    crate::session_cli::attach(
                        data_dir,
                        no_autostart,
                        session_id,
                        created.conversation.conversation_id.clone(),
                    )
                    .await
                    .map_err(|error| error.to_string())?,
                );
            }
            Ok(created)
        }
        terminal::UiRequest::Send {
            conversation_id,
            text,
            attachments,
        } => {
            let accepted = chat_cli::send(
                data_dir,
                no_autostart,
                conversation_id,
                text,
                attachments,
                Vec::new(),
            )
            .await
            .map_err(|error| error.to_string())?;
            let mut result = result(
                accepted.conversation_id.0,
                Some(accepted.run_id.0),
                delivery_notice(accepted.delivery),
            );
            result.awaiting_turn = Some(matches!(
                accepted.delivery,
                gent_types::AgentChatPromptDelivery::AwaitingReadiness
                    | gent_types::AgentChatPromptDelivery::AwaitingProvider
            ));
            Ok(result)
        }
        terminal::UiRequest::RunAutomation {
            automation_id,
            conversation_id,
        } => automation::run(data_dir, no_autostart, automation_id, conversation_id).await,
        terminal::UiRequest::Goal {
            conversation_id,
            run_id,
            summary,
        } => {
            crate::goal_cli::create_shorthand(
                data_dir,
                no_autostart,
                conversation_id.clone(),
                run_id,
                summary,
            )
            .await
            .map_err(|error| error.to_string())?;
            Ok(result(
                conversation_id,
                None,
                "Goal saved; it will be projected only by an authorized provider turn.",
            ))
        }
        terminal::UiRequest::Switch {
            conversation_id,
            parent_run_id,
            selection,
            context_policy,
        } => {
            let run_id = chat_cli::switch::request(
                data_dir,
                no_autostart,
                conversation_id.clone(),
                parent_run_id,
                selection,
                context_policy,
            )
            .await
            .map_err(|error| error.to_string())?;
            Ok(result(
                conversation_id,
                Some(run_id.0),
                "Selection switched; prompts now target the new durable run.",
            ))
        }
        terminal::UiRequest::Permission { response } => {
            let conversation_id = response.binding.conversation_id.0.clone();
            crate::permissions_cli::agent_chat::respond(data_dir, no_autostart, response)
                .await
                .map_err(|error| error.to_string())?;
            Ok(result(
                conversation_id,
                None,
                "Permission response saved; Gentd will relay it to the provider.",
            ))
        }
        terminal::UiRequest::SetPermissionMode {
            conversation_id,
            workspace_id,
            mode,
            bypass_consent,
        } => {
            crate::permissions_cli::set_mode(
                data_dir,
                no_autostart,
                workspace_id,
                mode,
                bypass_consent,
            )
            .await
            .map_err(|error| error.to_string())?;
            let mut saved = result(
                conversation_id,
                None,
                "Permission posture saved for this workspace.",
            );
            saved.permission_mode = Some(mode);
            Ok(saved)
        }
        terminal::UiRequest::Interrupt {
            conversation_id,
            run_id,
        } => {
            chat_cli::interrupt(data_dir, no_autostart, conversation_id.clone(), run_id)
                .await
                .map_err(|error| error.to_string())?;
            let mut result = result(conversation_id, None, "Canceled current work.");
            result.awaiting_turn = Some(false);
            Ok(result)
        }
    }
}
