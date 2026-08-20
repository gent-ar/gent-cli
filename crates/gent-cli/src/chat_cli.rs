//! Typed `gent chat` requests over negotiated local agent-chat IPC.

use std::path::PathBuf;

use crate::local_ipc::connect_and_negotiate;
use clap::Subcommand;
use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{AgentChatConversationId, AgentChatRequestId, AgentChatSelection, ReceiptId};
use serde_json::Value;

mod arguments;
pub(crate) mod follow;
mod prompt;
mod reads;
mod resume;
mod selection;
pub(crate) mod switch;
pub(crate) mod turn_follow;
pub(crate) use arguments::{
    ConversationArgs, CreateArgs, DirectPromptArgs, Effort, Mode, PromptArgs, Provider,
    TranscriptArgs,
};
pub(crate) use prompt::send;
pub(crate) use reads::transcript;
pub(crate) use selection::{effort, mode, provider};

#[derive(Debug, Subcommand)]
pub(crate) enum ChatCommand {
    Create(CreateArgs),
    Send(PromptArgs),
    Resume(resume::ResumeArgs),
    Queue(PromptArgs),
    Switch(switch::SwitchArgs),
    /// Follow daemon-normalized transcript events, resuming from a durable cursor after reconnect.
    Follow(follow::FollowArgs),
    /// Follow exactly one normalized durable turn through its terminal record.
    FollowTurn(turn_follow::FollowTurnArgs),
    /// Read one provider-neutral conversation summary.
    Summary(ConversationArgs),
    /// Read one provider-neutral conversation and its normalized run hierarchy.
    Detail(ConversationArgs),
    /// Read one bounded page of daemon-normalized transcript events.
    Transcript(TranscriptArgs),
}

/// Executes one short-lived agent-chat command and returns its public JSON response.
pub(crate) async fn execute_command(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    action: ChatCommand,
) -> Result<Value, Box<dyn std::error::Error>> {
    let value = match action {
        ChatCommand::Follow(_) | ChatCommand::FollowTurn(_) => {
            return Err("chat follow is a long-lived subscription".into());
        }
        ChatCommand::Summary(args) => reads::summary(data_dir, no_autostart, args.conversation_id)
            .await
            .and_then(to_value)?,
        ChatCommand::Detail(args) => reads::detail(data_dir, no_autostart, args.conversation_id)
            .await
            .and_then(to_value)?,
        ChatCommand::Transcript(args) => reads::transcript(
            data_dir,
            no_autostart,
            args.conversation_id,
            args.after_cursor,
            args.limit,
        )
        .await
        .and_then(to_value)?,
        action => execute(data_dir, no_autostart, action)
            .await
            .and_then(to_value)?,
    };
    Ok(value)
}

fn to_value(value: impl serde::Serialize) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::to_value(value)?)
}

/// Exchanges exactly one capability-gated agent-chat intent with the local daemon.
pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    action: ChatCommand,
) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    if matches!(
        &action,
        ChatCommand::Follow(_)
            | ChatCommand::FollowTurn(_)
            | ChatCommand::Summary(_)
            | ChatCommand::Detail(_)
            | ChatCommand::Transcript(_)
    ) {
        return Err("agent-chat reads and follow bypass one-shot intent frames".into());
    }
    exchange(data_dir, no_autostart, frame(action)?).await
}

/// Creates a selected conversation for an interactive terminal through the same IPC boundary.
pub(crate) async fn create(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    selection: AgentChatSelection,
    workspace: Option<PathBuf>,
) -> Result<(AgentChatConversationId, gent_types::AgentChatRunId), Box<dyn std::error::Error>> {
    let response = exchange(
        data_dir,
        no_autostart,
        AgentChatIntentFrame::CreateConversation {
            request_id: request_id(None),
            receipt_id: receipt_id(None),
            workspace_path: workspace_path(workspace)?,
            selection,
        },
    )
    .await?;
    let AgentChatIntentFrame::Created {
        conversation_id,
        run_id,
        ..
    } = response
    else {
        return Err("daemon did not return a created conversation".into());
    };
    Ok((conversation_id, run_id))
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: AgentChatIntentFrame,
) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    if !capabilities
        .0
        .iter()
        .any(|item| item == AGENT_CHAT_INTENTS_CAPABILITY)
    {
        return Err("daemon does not support agent chat; upgrade gentd".into());
    }
    write_json_frame(&mut stream, &request).await?;
    let raw: Value = read_json_frame(&mut stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    let response =
        serde_json::from_value(raw).map_err(|_| "daemon did not return an agent-chat response")?;
    valid_reply(&request, &response)
        .then_some(response)
        .ok_or_else(|| {
            "daemon returned an agent-chat response with a different request or receipt".into()
        })
}

fn frame(action: ChatCommand) -> Result<AgentChatIntentFrame, Box<dyn std::error::Error>> {
    Ok(match action {
        ChatCommand::Create(args) => AgentChatIntentFrame::CreateConversation {
            request_id: request_id(args.request_id),
            receipt_id: receipt_id(args.receipt_id),
            workspace_path: workspace_path(args.workspace)?,
            selection: AgentChatSelection {
                provider: provider(args.provider),
                model: args.model,
                effort: effort(args.effort),
                mode: mode(args.mode),
            },
        },
        ChatCommand::Send(args) => prompt_frame(args, false),
        ChatCommand::Resume(args) => resume::frame(args),
        ChatCommand::Queue(args) => prompt_frame(args, true),
        ChatCommand::Switch(args) => switch::frame(args),
        ChatCommand::Follow(_) => unreachable!("long-lived subscriptions bypass one-shot frames"),
        ChatCommand::FollowTurn(_) => unreachable!("turn follow bypasses one-shot frames"),
        ChatCommand::Summary(_) | ChatCommand::Detail(_) | ChatCommand::Transcript(_) => {
            unreachable!("agent-chat reads bypass intent frames")
        }
    })
}

fn workspace_path(value: Option<PathBuf>) -> Result<String, Box<dyn std::error::Error>> {
    value
        .unwrap_or(std::env::current_dir()?)
        .into_os_string()
        .into_string()
        .map_err(|_| "workspace path must be valid UTF-8".into())
}

fn prompt_frame(args: PromptArgs, queued: bool) -> AgentChatIntentFrame {
    let value = (
        request_id(args.request_id),
        receipt_id(args.receipt_id),
        AgentChatConversationId(args.conversation_id),
        args.text,
    );
    if queued {
        AgentChatIntentFrame::QueuePrompt {
            request_id: value.0,
            receipt_id: value.1,
            conversation_id: value.2,
            text: value.3,
        }
    } else {
        AgentChatIntentFrame::SendPrompt {
            request_id: value.0,
            receipt_id: value.1,
            conversation_id: value.2,
            text: value.3,
        }
    }
}

fn valid_reply(request: &AgentChatIntentFrame, response: &AgentChatIntentFrame) -> bool {
    if let Some(valid) = switch::valid_reply(request, response) {
        return valid;
    }
    match (request, response) {
        (
            AgentChatIntentFrame::CreateConversation {
                request_id,
                receipt_id,
                ..
            },
            AgentChatIntentFrame::Created {
                request_id: reply,
                receipt,
                conversation_id,
                run_id,
            },
        ) => {
            reply == request_id
                && receipt.receipt_id == *receipt_id
                && !conversation_id.0.is_empty()
                && !run_id.0.is_empty()
        }
        (
            AgentChatIntentFrame::SendPrompt {
                request_id,
                receipt_id,
                conversation_id: expected_conversation_id,
                ..
            }
            | AgentChatIntentFrame::QueuePrompt {
                request_id,
                receipt_id,
                conversation_id: expected_conversation_id,
                ..
            },
            AgentChatIntentFrame::Accepted {
                request_id: reply,
                receipt,
                conversation_id,
                run_id,
                turn_id,
                ..
            },
        ) => {
            reply == request_id
                && receipt.receipt_id == *receipt_id
                && conversation_id == expected_conversation_id
                && !conversation_id.0.is_empty()
                && !run_id.0.is_empty()
                && !turn_id.is_empty()
        }
        _ => false,
    }
}

fn request_id(value: Option<String>) -> AgentChatRequestId {
    AgentChatRequestId(value.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()))
}
fn receipt_id(value: Option<String>) -> ReceiptId {
    value.map_or_else(ReceiptId::new, ReceiptId)
}
#[cfg(all(test, unix))]
#[path = "chat_cli/resume_tests.rs"]
mod resume_tests;
#[cfg(all(test, unix))]
#[path = "chat_cli/tests.rs"]
mod tests;
