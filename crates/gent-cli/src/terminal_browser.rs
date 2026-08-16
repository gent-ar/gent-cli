//! Terminal composition edge: it connects UI requests to protocol-only agent-chat IPC.

use std::path::PathBuf;

use gent_protocol::AGENT_CHAT_INTENTS_CAPABILITY;
use gent_types::ConversationListItem;

use crate::{chat_cli, conversation_index, local_ipc, terminal};

/// Opens the terminal after negotiation has established the available authority profile.
pub(crate) async fn open(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    terminal::require_interactive()?;
    let index = conversation_index::request(data_dir.clone(), no_autostart).await?;
    let (_, capabilities) =
        local_ipc::connect_and_negotiate(data_dir.clone(), no_autostart).await?;
    let enabled = capabilities
        .0
        .iter()
        .any(|value| value == AGENT_CHAT_INTENTS_CAPABILITY);
    let runtime = tokio::runtime::Handle::current();
    terminal::run(
        terminal::UiState::new(index).with_chat_input(enabled),
        move |intent| submit(&runtime, data_dir.clone(), no_autostart, intent),
    )?;
    Ok(())
}

fn submit(
    runtime: &tokio::runtime::Handle,
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: terminal::UiRequest,
) -> Result<ConversationListItem, String> {
    tokio::task::block_in_place(|| {
        runtime.block_on(async move {
            match request {
                terminal::UiRequest::Create { selection } => {
                    let (conversation_id, _) = chat_cli::create(data_dir, no_autostart, selection)
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok(ConversationListItem {
                        conversation_id: conversation_id.0,
                        run_count: 1,
                    })
                }
                terminal::UiRequest::Send {
                    conversation_id,
                    text,
                } => {
                    chat_cli::send(data_dir, no_autostart, conversation_id.clone(), text)
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok(ConversationListItem {
                        conversation_id,
                        run_count: 1,
                    })
                }
            }
        })
    })
}
