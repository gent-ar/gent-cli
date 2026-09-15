use std::path::PathBuf;

use clap::Args;
use gent_protocol::AgentChatIntentFrame;
use gent_types::AgentChatConversationId;

use crate::{
    cli_error::{CliError, Failure},
    conversation_activity, conversation_status,
    prompt_queue::{QueuedPrompt, queued_prompts},
};

#[derive(Debug, Args)]
pub(crate) struct QueuedArgs {
    #[arg(long, help = "Conversation whose queued prompts are listed")]
    pub(crate) conversation_id: String,
}

#[derive(Debug, Args)]
pub(crate) struct SteerArgs {
    #[arg(long, help = "Conversation with the running turn")]
    pub(crate) conversation_id: String,
    #[arg(
        long,
        help = "Queued prompt to deliver now [default: every queued prompt, oldest first]"
    )]
    pub(crate) message_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct CancelQueuedArgs {
    #[arg(long, help = "Conversation whose queued prompt is removed")]
    pub(crate) conversation_id: String,
    #[arg(
        long,
        required_unless_present = "all",
        conflicts_with = "all",
        help = "Queued prompt to remove (see `gent chat queued`)"
    )]
    pub(crate) message_id: Option<String>,
    #[arg(long, help = "Remove every queued prompt")]
    pub(crate) all: bool,
}

pub(crate) async fn list(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
) -> Result<Vec<QueuedPrompt>, Box<dyn std::error::Error>> {
    let status =
        conversation_status::request(data_dir.clone(), no_autostart, conversation_id.clone())
            .await?;
    let mut facts = Vec::new();
    for run in status.runs {
        facts.extend(
            conversation_activity::all(
                data_dir.clone(),
                no_autostart,
                conversation_id.clone(),
                run.run_id,
            )
            .await?,
        );
    }
    Ok(queued_prompts(&facts))
}

pub(crate) async fn steer(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: SteerArgs,
) -> Result<Vec<AgentChatIntentFrame>, Box<dyn std::error::Error>> {
    let message_ids = match args.message_id {
        Some(message_id) => vec![message_id],
        None => queued_ids(data_dir.clone(), no_autostart, &args.conversation_id).await?,
    };
    deliver(
        data_dir,
        no_autostart,
        &args.conversation_id,
        message_ids,
        true,
    )
    .await
}

pub(crate) async fn cancel(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: CancelQueuedArgs,
) -> Result<Vec<AgentChatIntentFrame>, Box<dyn std::error::Error>> {
    let message_ids = match args.message_id {
        Some(message_id) => vec![message_id],
        None => queued_ids(data_dir.clone(), no_autostart, &args.conversation_id).await?,
    };
    deliver(
        data_dir,
        no_autostart,
        &args.conversation_id,
        message_ids,
        false,
    )
    .await
}

pub(crate) async fn deliver(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &str,
    message_ids: Vec<String>,
    steer: bool,
) -> Result<Vec<AgentChatIntentFrame>, Box<dyn std::error::Error>> {
    let mut replies = Vec::with_capacity(message_ids.len());
    for message_id in message_ids {
        replies.push(
            super::exchange(
                data_dir.clone(),
                no_autostart,
                frame(conversation_id, message_id, steer),
            )
            .await?,
        );
    }
    Ok(replies)
}

async fn queued_ids(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let ids = list(data_dir, no_autostart, conversation_id.to_owned())
        .await?
        .into_iter()
        .map(|prompt| prompt.message_id)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Err(CliError::new(
            Failure::NotFound,
            "no prompts are queued in this conversation",
        )
        .into());
    }
    Ok(ids)
}

fn frame(conversation_id: &str, message_id: String, steer: bool) -> AgentChatIntentFrame {
    let conversation_id = AgentChatConversationId(conversation_id.to_owned());
    if steer {
        AgentChatIntentFrame::SteerQueuedPrompt {
            request_id: super::request_id(None),
            receipt_id: super::receipt_id(None),
            conversation_id,
            message_id,
        }
    } else {
        AgentChatIntentFrame::CancelQueuedPrompt {
            request_id: super::request_id(None),
            receipt_id: super::receipt_id(None),
            conversation_id,
            message_id,
        }
    }
}

pub(crate) fn valid_reply(
    request: &AgentChatIntentFrame,
    response: &AgentChatIntentFrame,
) -> Option<bool> {
    match (request, response) {
        (
            AgentChatIntentFrame::SteerQueuedPrompt {
                request_id,
                receipt_id,
                conversation_id,
                message_id,
            },
            AgentChatIntentFrame::QueuedPromptSteered {
                request_id: reply,
                receipt,
                conversation_id: reply_conversation,
                message_id: reply_message,
            },
        )
        | (
            AgentChatIntentFrame::CancelQueuedPrompt {
                request_id,
                receipt_id,
                conversation_id,
                message_id,
            },
            AgentChatIntentFrame::QueuedPromptCanceled {
                request_id: reply,
                receipt,
                conversation_id: reply_conversation,
                message_id: reply_message,
            },
        ) => Some(
            reply == request_id
                && receipt.receipt_id == *receipt_id
                && reply_conversation == conversation_id
                && reply_message == message_id,
        ),
        (
            AgentChatIntentFrame::SteerQueuedPrompt { .. }
            | AgentChatIntentFrame::CancelQueuedPrompt { .. },
            _,
        ) => Some(false),
        _ => None,
    }
}

#[cfg(all(test, unix))]
#[path = "queue_tests.rs"]
mod tests;
