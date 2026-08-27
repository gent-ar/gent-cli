//! `gent chat` command dispatch separate from the top-level command composition.

use std::path::PathBuf;

use gent_protocol::AgentChatIntentFrame;

use crate::chat_cli;

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    action: chat_cli::ChatCommand,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    match action {
        chat_cli::ChatCommand::Follow(args) => {
            chat_cli::follow::run(data_dir, no_autostart, args).await?;
            Ok(None)
        }
        chat_cli::ChatCommand::FollowTurn(args) => {
            chat_cli::turn_follow::run(data_dir, no_autostart, args).await?;
            Ok(None)
        }
        action @ (chat_cli::ChatCommand::Send(_) | chat_cli::ChatCommand::Resume(_)) => {
            let reply = chat_cli::execute(data_dir.clone(), no_autostart, action).await?;
            crate::command_execution::print(&reply)?;
            if let AgentChatIntentFrame::Accepted {
                conversation_id,
                run_id,
                turn_id,
                ..
            } = reply
            {
                let _ = chat_cli::turn_follow::follow_accepted_if_supported(
                    data_dir,
                    no_autostart,
                    conversation_id.0,
                    run_id.0,
                    turn_id,
                )
                .await?;
            }
            Ok(None)
        }
        action => Ok(Some(
            chat_cli::execute_command(data_dir, no_autostart, action).await?,
        )),
    }
}
