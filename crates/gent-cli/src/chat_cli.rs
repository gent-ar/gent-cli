//! Typed `gent chat` requests over negotiated local agent-chat IPC.

use std::path::PathBuf;

use crate::local_ipc::connect_and_negotiate;
use clap::Subcommand;
use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, WireFrame, read_json_frame,
    write_json_frame,
};
use gent_types::{AgentChatConversationId, AgentChatSelection};
use serde_json::Value;

mod arguments;
mod attachments;
pub(crate) mod follow;
mod intent;
mod interrupt;
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
pub(crate) use intent::{frame, prompt_frame, receipt_id, request_id, valid_reply, workspace_path};
pub(crate) use prompt::send;
pub(crate) use reads::{detail, summary, transcript_all};
pub(crate) use selection::{effort, mode, model, provider};

#[derive(Debug, Subcommand)]
pub(crate) enum ChatCommand {
    Create(CreateArgs),
    Send(PromptArgs),
    Resume(resume::ResumeArgs),
    Queue(PromptArgs),
    Interrupt(interrupt::InterruptArgs),
    Switch(switch::SwitchArgs),
    Fork(switch::SwitchArgs),
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
    let request = match action {
        ChatCommand::Switch(args) | ChatCommand::Fork(args) => {
            switch::resolve(data_dir.clone(), no_autostart, args).await?
        }
        ChatCommand::Send(args) => {
            let attachments =
                attachments::stage(data_dir.clone(), no_autostart, &args.attachments).await?;
            prompt_frame(args, false, attachments)
        }
        ChatCommand::Queue(args) => {
            let attachments =
                attachments::stage(data_dir.clone(), no_autostart, &args.attachments).await?;
            prompt_frame(args, true, attachments)
        }
        ChatCommand::Resume(args) => {
            let attachments =
                attachments::stage(data_dir.clone(), no_autostart, &args.attachments).await?;
            resume::frame(args, attachments)
        }
        action => frame(action)?,
    };
    exchange(data_dir, no_autostart, request).await
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

pub(crate) async fn interrupt(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    run_id: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = exchange(
        data_dir,
        no_autostart,
        AgentChatIntentFrame::Interrupt {
            request_id: request_id(None),
            receipt_id: receipt_id(None),
            conversation_id: AgentChatConversationId(conversation_id),
            run_id: gent_types::AgentChatRunId(run_id),
        },
    )
    .await?;
    if matches!(response, AgentChatIntentFrame::Interrupted { .. }) {
        Ok(())
    } else {
        Err("daemon did not confirm the interrupt".into())
    }
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

#[cfg(all(test, unix))]
#[path = "chat_cli/resume_tests.rs"]
mod resume_tests;
#[cfg(all(test, unix))]
#[path = "chat_cli/tests.rs"]
mod tests;
