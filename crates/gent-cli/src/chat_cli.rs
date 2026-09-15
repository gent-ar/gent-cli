//! Typed `gent chat` requests over negotiated local agent-chat IPC.

use std::path::PathBuf;

use crate::local_ipc::connect_and_negotiate;
use clap::Subcommand;
use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, read_json_frame, write_json_frame,
};
use gent_types::{AgentChatConversationId, AgentChatSelection};
use serde_json::Value;

mod arguments;
mod attachments;
pub(crate) mod continuation;
pub(crate) mod follow;
mod intent;
mod interrupt;
mod prompt;
pub(crate) mod queue;
mod reads;
mod resume;
mod selection;
pub(crate) mod switch;
pub(crate) mod turn_follow;
mod turn_view;
pub(crate) use turn_view::outcome as turn_view_outcome;
pub(crate) mod turn_watch;
pub(crate) use arguments::{
    ConversationArgs, CreateArgs, DirectPromptArgs, Mode, PromptArgs, Provider, SelectionArgs,
    TranscriptArgs,
};
pub(crate) use intent::{frame, prompt_frame, receipt_id, request_id, valid_reply, workspace_path};
pub(crate) use prompt::send;
pub(crate) use reads::{detail, summary, transcript_all};
pub(crate) use selection::{
    SelectionRequest, default_model, fitted_effort, parse_effort, provider_name,
};

#[derive(Debug, Subcommand)]
pub(crate) enum ChatCommand {
    #[command(about = "Create a conversation in a workspace and print its ids as JSON")]
    Create(CreateArgs),
    #[command(about = "Send a prompt and stream the reply until the turn ends")]
    Send(PromptArgs),
    #[command(about = "Send a prompt to an existing conversation and stream the reply")]
    Resume(resume::ResumeArgs),
    #[command(about = "Queue a prompt to run after the current turn and print its message id")]
    Queue(PromptArgs),
    #[command(about = "List prompts waiting in a conversation's queue")]
    Queued(queue::QueuedArgs),
    #[command(about = "Deliver queued prompts into the running turn now")]
    Steer(queue::SteerArgs),
    #[command(about = "Remove queued prompts before they run")]
    CancelQueued(queue::CancelQueuedArgs),
    #[command(about = "Stop the running turn of a conversation")]
    Interrupt(interrupt::InterruptArgs),
    #[command(
        about = "Send a turn again from Gent's saved history after its provider session was lost"
    )]
    ContinueFromHistory(continuation::ContinueFromHistoryArgs),
    #[command(about = "Change provider, model, effort, or mode for the next prompts")]
    Switch(switch::SwitchArgs),
    #[command(about = "Branch a new run with a different selection from a chosen run")]
    Fork(switch::SwitchArgs),
    #[command(about = "Print a conversation's transcript events as JSON lines and keep following")]
    Follow(follow::FollowArgs),
    #[command(about = "Print one turn's transcript events as JSON lines until it ends")]
    FollowTurn(turn_follow::FollowTurnArgs),
    #[command(about = "Print a conversation summary as JSON")]
    Summary(ConversationArgs),
    #[command(about = "Print a conversation with its runs and selections as JSON")]
    Detail(ConversationArgs),
    #[command(about = "Print one page of transcript events as JSON")]
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
        ChatCommand::Queued(args) => queue::list(data_dir, no_autostart, args.conversation_id)
            .await
            .and_then(to_value)?,
        ChatCommand::Steer(args) => queue::steer(data_dir, no_autostart, args)
            .await
            .and_then(to_value)?,
        ChatCommand::CancelQueued(args) => queue::cancel(data_dir, no_autostart, args)
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
            | ChatCommand::Queued(_)
            | ChatCommand::Steer(_)
            | ChatCommand::CancelQueued(_)
    ) {
        return Err("agent-chat reads and follow bypass one-shot intent frames".into());
    }
    let request = match action {
        ChatCommand::Switch(args) | ChatCommand::Fork(args) => {
            switch::resolve(data_dir.clone(), no_autostart, args).await?
        }
        ChatCommand::Interrupt(args) => {
            interrupt::resolve(data_dir.clone(), no_autostart, args).await?
        }
        ChatCommand::Create(args) => AgentChatIntentFrame::CreateConversation {
            request_id: request_id(args.request_id),
            receipt_id: receipt_id(args.receipt_id),
            workspace_path: workspace_path(args.workspace)?,
            selection: new_selection(data_dir.clone(), no_autostart, args.selection.request())
                .await?,
        },
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

pub(crate) fn turn_outcome(
    phase: gent_types::DurableTurnPhase,
) -> Result<(), crate::cli_error::CliError> {
    turn_view_outcome(phase, None, None)
}

pub(crate) async fn new_selection(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: SelectionRequest,
) -> Result<Option<AgentChatSelection>, Box<dyn std::error::Error>> {
    if request.is_empty() {
        return Ok(None);
    }
    let catalog = selection_catalog(data_dir, no_autostart, &request, None).await?;
    Ok(Some(request.resolve(None, catalog.as_ref())?))
}

pub(crate) async fn selection_catalog(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: &SelectionRequest,
    base: Option<&AgentChatSelection>,
) -> Result<Option<gent_protocol::model_catalog::ModelCatalog>, Box<dyn std::error::Error>> {
    if request.needs_catalog(base) {
        return crate::model_catalog_cli::read(data_dir, no_autostart)
            .await
            .map(Some);
    }
    if request.wants_catalog(base) {
        return crate::model_catalog_cli::read_if_exposed(data_dir, no_autostart).await;
    }
    Ok(None)
}

/// Creates a selected conversation for an interactive terminal through the same IPC boundary.
pub(crate) async fn create(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    selection: Option<AgentChatSelection>,
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
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.into());
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
