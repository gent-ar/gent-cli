use std::path::PathBuf;

use crate::chat_cli;
use crate::terminal;
use crate::terminal_browser::result::{delivery_notice, result};

pub(super) async fn create(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    selection: Option<gent_types::AgentChatSelection>,
    session_id: Option<gent_types::AgentChatSessionId>,
) -> Result<terminal::UiRequestResult, String> {
    let workspace = std::env::current_dir()
        .map_err(|_| "Gent could not determine the current workspace.".to_owned())?;
    if let Some(selection) = &selection {
        crate::model_catalog_cli::set_default(data_dir.clone(), no_autostart, selection)
            .await
            .map_err(|error| error.to_string())?;
    }
    let (conversation_id, run_id) =
        chat_cli::create(data_dir.clone(), no_autostart, None, Some(workspace))
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

pub(super) async fn send(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    text: String,
    attachments: Vec<PathBuf>,
    queued: bool,
) -> Result<terminal::UiRequestResult, String> {
    let accepted = chat_cli::send(
        data_dir,
        no_autostart,
        conversation_id,
        text,
        attachments,
        queued,
    )
    .await
    .map_err(|error| error.to_string())?;
    let mut sent = result(
        accepted.conversation_id.0,
        Some(accepted.run_id.0),
        delivery_notice(accepted.delivery),
    );
    sent.awaiting_turn = Some(matches!(
        accepted.delivery,
        gent_types::AgentChatPromptDelivery::AwaitingReadiness
            | gent_types::AgentChatPromptDelivery::AwaitingProvider
    ));
    Ok(sent)
}

pub(super) async fn switch(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    parent_run_id: String,
    selection: gent_types::AgentChatSelection,
    context_policy: gent_types::ContextPolicy,
) -> Result<terminal::UiRequestResult, String> {
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

pub(super) async fn permission(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    response: gent_types::PermissionDecisionResponse,
) -> Result<terminal::UiRequestResult, String> {
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

pub(super) async fn set_permission_mode(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    workspace_id: String,
    mode: gent_types::PermissionMode,
    bypass_consent: bool,
) -> Result<terminal::UiRequestResult, String> {
    crate::permissions_cli::set_mode(data_dir, no_autostart, workspace_id, mode, bypass_consent)
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

pub(super) async fn steer_queued(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    message_ids: Vec<String>,
) -> Result<terminal::UiRequestResult, String> {
    let count = message_ids.len();
    chat_cli::queue::deliver(data_dir, no_autostart, &conversation_id, message_ids, true)
        .await
        .map_err(|error| error.to_string())?;
    Ok(result(
        conversation_id,
        None,
        if count == 1 {
            "Sent the queued prompt into the running turn.".to_owned()
        } else {
            format!("Sent {count} queued prompts into the running turn.")
        },
    ))
}

pub(super) async fn cancel_queued(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    message_id: String,
) -> Result<terminal::UiRequestResult, String> {
    chat_cli::queue::deliver(
        data_dir,
        no_autostart,
        &conversation_id,
        vec![message_id],
        false,
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(result(
        conversation_id,
        None,
        "Removed the last queued prompt.",
    ))
}

pub(super) async fn continue_from_history(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    message_id: String,
) -> Result<terminal::UiRequestResult, String> {
    let accepted =
        chat_cli::continuation::send(data_dir, no_autostart, conversation_id, message_id)
            .await
            .map_err(|error| error.to_string())?;
    let mut continued = result(
        accepted.conversation_id.0,
        Some(accepted.run_id.0),
        "Continuing from Gent's saved history.",
    );
    continued.awaiting_turn = Some(true);
    Ok(continued)
}

pub(super) async fn interrupt(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    run_id: String,
) -> Result<terminal::UiRequestResult, String> {
    chat_cli::interrupt(data_dir, no_autostart, conversation_id.clone(), run_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut interrupted = result(conversation_id, None, "Canceled current work.");
    interrupted.awaiting_turn = Some(false);
    Ok(interrupted)
}
