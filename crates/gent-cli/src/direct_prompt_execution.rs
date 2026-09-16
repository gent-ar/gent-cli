use std::path::PathBuf;

use crate::{
    chat_cli::{DirectPromptArgs, turn_watch},
    command_execution::print,
    direct_prompt::{self, DirectPromptResult},
};

const CATALOG_LOADING_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
const CATALOG_LOADING_POLL: std::time::Duration = std::time::Duration::from_millis(500);

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: DirectPromptArgs,
) -> Result<bool, Box<dyn std::error::Error>> {
    let json = args.json;
    let Some(reply) = direct_prompt::execute(data_dir.clone(), no_autostart, args).await? else {
        return Ok(false);
    };
    report(data_dir, no_autostart, &reply, json).await?;
    Ok(true)
}

pub(crate) async fn report(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    reply: &DirectPromptResult,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let DirectPromptResult::Prompt {
        conversation_id,
        run_id,
        turn_id,
        delivery,
        ..
    } = reply
    else {
        return print(reply);
    };
    if json {
        print(reply)?;
    } else {
        turn_watch::announce_delivery(data_dir.as_deref(), conversation_id, *delivery);
    }
    let target = turn_watch::TurnTarget {
        conversation: conversation_id.clone(),
        run: run_id.clone(),
        turn: turn_id.clone(),
    };
    turn_watch::follow(data_dir.clone(), no_autostart, &target, json).await?;
    if !json {
        eprintln!(
            "Continue this conversation: {} --conversation-id {} \"<prompt>\"",
            turn_watch::command_prefix(data_dir.as_deref()),
            target.conversation
        );
    }
    Ok(())
}

pub(crate) async fn invoke_and_report(
    (data_dir, no_autostart): (Option<PathBuf>, bool),
    conversation_id: String,
    receipt_id: Option<String>,
    text: &str,
    attachments: &[PathBuf],
    json: bool,
) -> Result<bool, Box<dyn std::error::Error>> {
    let Some(reply) = command(
        data_dir.clone(),
        no_autostart,
        Some(conversation_id),
        receipt_id,
        text,
        attachments,
    )
    .await?
    else {
        return Ok(false);
    };
    report(data_dir, no_autostart, &reply, json).await?;
    Ok(true)
}

pub(crate) async fn command(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: Option<String>,
    receipt_id: Option<String>,
    text: &str,
    attachments: &[PathBuf],
) -> Result<Option<DirectPromptResult>, Box<dyn std::error::Error>> {
    let Some((name, arguments)) = gent_types::slash_command(text) else {
        return Ok(None);
    };
    if !attachments.is_empty() {
        return Err(format!("/{name} does not take attachments; nothing was sent").into());
    }
    let receipt_id = receipt_id.unwrap_or_else(|| gent_types::ReceiptId::new().0);
    let mut waited = std::time::Duration::ZERO;
    let (receipt, outcome) = loop {
        match crate::command_catalog_cli::invoke(
            data_dir.clone(),
            no_autostart,
            conversation_id.clone(),
            Some(receipt_id.clone()),
            name.into(),
            arguments.into(),
        )
        .await
        {
            Err(error) if waited < CATALOG_LOADING_WAIT && catalog_loading(error.as_ref()) => {
                tokio::time::sleep(CATALOG_LOADING_POLL).await;
                waited += CATALOG_LOADING_POLL;
            }
            result => break result?,
        }
    };
    Ok(Some(match (outcome, conversation_id) {
        (
            gent_protocol::agent_chat_commands::CommandOutcome::Delivered {
                run_id,
                turn_id,
                delivery,
                ..
            },
            Some(conversation_id),
        ) => DirectPromptResult::Prompt {
            conversation_id,
            run_id: run_id.0,
            turn_id,
            prompt_receipt_id: receipt.receipt_id.0,
            delivery,
        },
        (outcome, conversation_id) => DirectPromptResult::Command {
            conversation_id,
            outcome,
        },
    }))
}

fn catalog_loading(error: &(dyn std::error::Error + 'static)) -> bool {
    error
        .downcast_ref::<crate::cli_error::CliError>()
        .and_then(crate::cli_error::CliError::code)
        == Some("commandCatalogLoading")
}
