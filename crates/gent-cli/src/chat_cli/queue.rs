use std::path::PathBuf;

use clap::Args;
use gent_protocol::AgentChatIntentFrame;
use gent_types::{
    AgentChatConversationId, AgentChatRunId, ConversationRunStatus, ConversationStatus,
};

use crate::{
    cli_error::{CliError, Failure},
    conversation_activity, conversation_status,
    prompt_queue::{QueuedPrompt, queued_prompts},
};

pub(crate) use super::exchange::queue_valid_reply as valid_reply;

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
    list_for_status(data_dir, no_autostart, status).await
}

async fn list_for_status(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    status: ConversationStatus,
) -> Result<Vec<QueuedPrompt>, Box<dyn std::error::Error>> {
    let mut facts = Vec::new();
    for run in status.runs {
        facts.extend(
            conversation_activity::all(
                data_dir.clone(),
                no_autostart,
                status.conversation_id.clone(),
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
    let status =
        conversation_status::request(data_dir.clone(), no_autostart, args.conversation_id.clone())
            .await?;
    let target = steer_target(&status).ok_or("no active turn is available to steer")?;
    let queue = list_for_status(data_dir.clone(), no_autostart, status).await?;
    let message_ids = match args.message_id {
        Some(message_id) => explicit_steer_ids(&queue, message_id)?,
        None => select_ids(queue, true)?,
    };
    deliver(
        data_dir,
        no_autostart,
        &args.conversation_id,
        message_ids,
        Some(&target),
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
        None => select_ids(
            list(data_dir.clone(), no_autostart, args.conversation_id.clone()).await?,
            false,
        )?,
    };
    deliver(
        data_dir,
        no_autostart,
        &args.conversation_id,
        message_ids,
        None,
    )
    .await
}

pub(crate) async fn deliver(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &str,
    message_ids: Vec<String>,
    target: Option<&ConversationRunStatus>,
) -> Result<Vec<AgentChatIntentFrame>, Box<dyn std::error::Error>> {
    let required = target.map(|run| format!("agent-chat-steer-turn-{}-v1", run.provider));
    let capabilities = required.as_ref().map_or_else(Vec::new, |provider| {
        vec![
            gent_protocol::AGENT_CHAT_STEER_TURN_CAPABILITY,
            provider.as_str(),
        ]
    });
    let mut replies = Vec::with_capacity(message_ids.len());
    for message_id in message_ids {
        match super::exchange::required(
            data_dir.clone(),
            no_autostart,
            frame(conversation_id, message_id.clone(), target)?,
            &capabilities,
        )
        .await
        {
            Ok(reply) => replies.push(reply),
            Err(error) => {
                if target.is_some()
                    && queued_prompt_not_steerable(&*error)
                    && outstanding_steer_request(
                        data_dir.clone(),
                        no_autostart,
                        conversation_id,
                        &message_id,
                    )
                    .await?
                {
                    continue;
                }
                return Err(error);
            }
        }
    }
    Ok(replies)
}

async fn outstanding_steer_request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &str,
    message_id: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(list(data_dir, no_autostart, conversation_id.to_owned())
        .await?
        .iter()
        .any(|prompt| prompt.message_id == message_id && prompt.steer_receipt_id.is_some()))
}

fn queued_prompt_not_steerable(error: &(dyn std::error::Error + 'static)) -> bool {
    error
        .downcast_ref::<CliError>()
        .is_some_and(|error| error.code() == Some("queuedPromptNotSteerable"))
}

fn select_ids(
    queue: Vec<QueuedPrompt>,
    steer: bool,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let ids = queue
        .into_iter()
        .filter(|prompt| !steer || prompt.can_steer())
        .map(|prompt| prompt.message_id)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Err(CliError::new(
            Failure::NotFound,
            if steer {
                "no prompts are available to steer"
            } else {
                "no prompts are queued in this conversation"
            },
        )
        .into());
    }
    Ok(ids)
}

fn explicit_steer_ids(
    queue: &[QueuedPrompt],
    message_id: String,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let Some(prompt) = queue.iter().find(|prompt| prompt.message_id == message_id) else {
        return Ok(vec![message_id]);
    };
    if prompt.can_steer() {
        return Ok(vec![message_id]);
    }
    let message = if prompt.steer_receipt_id.is_some() {
        "a steer request is already outstanding for this message"
    } else {
        "delivery could not be proven for this message; use deliberate recovery before steering again"
    };
    Err(CliError::new(Failure::Rejected, message).into())
}

pub(crate) fn steer_target(status: &ConversationStatus) -> Option<ConversationRunStatus> {
    let mut active = status.runs.iter().filter(|run| {
        !run.run_id.is_empty()
            && run
                .active_turn_id
                .as_deref()
                .is_some_and(|turn| !turn.is_empty())
    });
    let target = active.next()?;
    active.next().is_none().then(|| target.clone())
}

fn frame(
    conversation_id: &str,
    message_id: String,
    target: Option<&ConversationRunStatus>,
) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    let conversation_id = AgentChatConversationId(conversation_id.to_owned());
    Ok(if let Some(target) = target {
        AgentChatIntentFrame::SteerQueuedPromptForTurn {
            request_id: super::request_id(None),
            receipt_id: super::receipt_id(None),
            conversation_id,
            message_id,
            expected_run_id: AgentChatRunId(target.run_id.clone()),
            expected_turn_id: target
                .active_turn_id
                .clone()
                .filter(|turn| !turn.is_empty())
                .ok_or("no active turn is available to steer")?,
        }
    } else {
        AgentChatIntentFrame::CancelQueuedPrompt {
            request_id: super::request_id(None),
            receipt_id: super::receipt_id(None),
            conversation_id,
            message_id,
        }
    })
}

#[cfg(all(test, unix))]
#[path = "queue_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "queue_target_tests.rs"]
mod target_tests;
