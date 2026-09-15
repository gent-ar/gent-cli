use std::path::PathBuf;

use gent_protocol::agent_chat_commands::CommandOutcome;
use gent_types::{AgentChatCommandIntent as Intent, AgentChatPromptDelivery, AgentChatSessionId};

use crate::{
    terminal,
    terminal_browser::result::{delivery_notice, result},
};

pub(super) async fn invoke(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: Option<String>,
    name: String,
    arguments: String,
    session_id: Option<AgentChatSessionId>,
) -> Result<terminal::UiRequestResult, String> {
    let (_, outcome) = crate::command_catalog_cli::invoke(
        data_dir.clone(),
        no_autostart,
        conversation_id.clone(),
        None,
        name,
        arguments,
    )
    .await
    .map_err(|error| error.to_string())?;
    match outcome {
        CommandOutcome::Delivered {
            run_id, delivery, ..
        } => {
            let mut delivered = result(
                conversation_id.unwrap_or_default(),
                Some(run_id.0),
                delivery_notice(delivery),
            );
            delivered.awaiting_turn = Some(matches!(
                delivery,
                AgentChatPromptDelivery::AwaitingReadiness
                    | AgentChatPromptDelivery::AwaitingProvider
            ));
            Ok(delivered)
        }
        CommandOutcome::IntentApplied {
            intent,
            conversation_id: target,
            run_id,
        } => {
            let mut applied = result(target.0, run_id.map(|run| run.0), notice(intent));
            if let (Intent::CreateConversation, Some(session_id)) = (intent, session_id) {
                applied.session = Some(
                    crate::session_cli::attach(
                        data_dir,
                        no_autostart,
                        session_id,
                        applied.conversation.conversation_id.clone(),
                    )
                    .await
                    .map_err(|error| error.to_string())?,
                );
            }
            Ok(applied)
        }
    }
}

const fn notice(intent: Intent) -> &'static str {
    match intent {
        Intent::CreateConversation => "Conversation created; choose a prompt to persist.",
        Intent::ForkConversation => "Conversation forked; prompts now continue in the fork.",
        Intent::ClearContext => "Context cleared; prompts now target a new run without history.",
        Intent::Goal => "Goal updated.",
        Intent::Compact => "Compacting the conversation context…",
        Intent::SelectProvider
        | Intent::SelectModel
        | Intent::SelectEffort
        | Intent::SelectMode => "Selection switched; prompts now target the new durable run.",
    }
}
