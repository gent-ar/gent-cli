//! Prompt-first terminal flow over the same typed local IPC as every other chat client.

use std::path::PathBuf;

use gent_types::{AgentChatConversationId, AgentChatSelection};
use serde::Serialize;

use crate::chat_cli::{self, DirectPromptArgs, effort, mode, provider};

/// Public terminal result after durable prompt submission; it never claims provider execution.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectPromptResult {
    conversation_id: String,
    run_id: Option<String>,
    delivery: gent_types::AgentChatPromptDelivery,
}

/// Creates a selected conversation when needed, then durably submits one terminal prompt.
pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: DirectPromptArgs,
) -> Result<Option<DirectPromptResult>, Box<dyn std::error::Error>> {
    let Some(text) = args.prompt else {
        return Ok(None);
    };
    let (conversation_id, run_id) = if let Some(conversation_id) = args.conversation_id {
        (AgentChatConversationId(conversation_id), None)
    } else {
        let selection = AgentChatSelection {
            provider: provider(args.provider),
            model: args.model,
            effort: effort(args.effort),
            mode: mode(args.mode),
        };
        let (conversation_id, run_id) =
            chat_cli::create(data_dir.clone(), no_autostart, selection).await?;
        (conversation_id, Some(run_id))
    };
    let delivery = chat_cli::send(data_dir, no_autostart, conversation_id.0.clone(), text).await?;
    Ok(Some(DirectPromptResult {
        conversation_id: conversation_id.0,
        run_id: run_id.map(|value| value.0),
        delivery,
    }))
}
