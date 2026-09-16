use std::path::PathBuf;

use gent_protocol::AgentChatIntentFrame;

use crate::chat_cli::{self, turn_watch};

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
            let terminal = chat_cli::turn_follow::run(
                data_dir,
                no_autostart,
                args,
                &mut chat_cli::turn_follow::print_json,
            )
            .await?;
            chat_cli::turn_outcome(terminal.phase)
                .map(|()| None)
                .map_err(Into::into)
        }
        chat_cli::ChatCommand::Send(args) => {
            let json = args.json;
            if args.tool_sources.is_empty()
                && crate::direct_prompt_execution::invoke_and_report(
                    (data_dir.clone(), no_autostart),
                    args.conversation_id.clone(),
                    args.receipt_id.clone(),
                    &args.text,
                    &args.attachments,
                    json,
                )
                .await?
            {
                return Ok(None);
            }
            prompt(
                data_dir,
                no_autostart,
                chat_cli::ChatCommand::Send(args),
                json,
            )
            .await
        }
        chat_cli::ChatCommand::ContinueFromHistory(args) => {
            let json = args.json;
            prompt(
                data_dir,
                no_autostart,
                chat_cli::ChatCommand::ContinueFromHistory(args),
                json,
            )
            .await
        }
        chat_cli::ChatCommand::Resume(args) => {
            let json = args.json;
            if crate::direct_prompt_execution::invoke_and_report(
                (data_dir.clone(), no_autostart),
                args.conversation_id.clone(),
                args.receipt_id.clone(),
                &args.text,
                &args.attachments,
                json,
            )
            .await?
            {
                return Ok(None);
            }
            prompt(
                data_dir,
                no_autostart,
                chat_cli::ChatCommand::Resume(args),
                json,
            )
            .await
        }
        action => Ok(Some(
            chat_cli::execute_command(data_dir, no_autostart, action).await?,
        )),
    }
}

async fn prompt(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    action: chat_cli::ChatCommand,
    json: bool,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    let reply = chat_cli::execute(data_dir.clone(), no_autostart, action).await?;
    if json {
        crate::command_execution::print(&reply)?;
    }
    if let AgentChatIntentFrame::Accepted {
        conversation_id,
        run_id,
        turn_id,
        delivery,
        ..
    } = reply
    {
        if !json {
            turn_watch::announce_delivery(data_dir.as_deref(), &conversation_id.0, delivery);
        }
        turn_watch::follow(
            data_dir,
            no_autostart,
            &turn_watch::TurnTarget {
                conversation: conversation_id.0,
                run: run_id.0,
                turn: turn_id,
            },
            json,
        )
        .await?;
    }
    Ok(None)
}
