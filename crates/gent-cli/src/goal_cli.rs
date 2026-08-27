//! Capability-gated terminal commands for durable, provider-neutral `/goal` records.
//!
//! This client only exchanges typed goal frames with local gentd. It never starts a
//! provider, derives a goal from provider output, or changes daemon authority.

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use gent_protocol::{
    GOAL_CAPABILITY, GoalFrame, WireFrame, read_json_frame, write_frame, write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, GOAL_SCHEMA_VERSION, GoalBinding, GoalRecord,
    GoalStatus, GoalTransition,
};
use serde_json::Value;

use crate::local_ipc::{LocalStream, connect_and_negotiate};

mod reply;
use reply::valid_reply;

/// Terminal-facing goal actions shared with any future native host client.
#[derive(Debug, Subcommand)]
pub(crate) enum GoalCommand {
    /// Create one active user-authored goal bound to an existing conversation run.
    Create(CreateArgs),
    /// Read one exact goal binding.
    Read(ReadArgs),
    /// List goals for one durable conversation.
    List(ListArgs),
    /// Revision-fenced terminal transition of one active goal.
    Transition(TransitionArgs),
}

#[derive(Debug, Args)]
pub(crate) struct CreateArgs {
    #[arg(long)]
    conversation_id: String,
    #[arg(long)]
    run_id: String,
    #[arg(long)]
    summary: String,
    #[arg(long)]
    goal_id: Option<String>,
    #[arg(long)]
    request_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ReadArgs {
    #[arg(long = "conversation-id")]
    conversation: String,
    #[arg(long = "run-id")]
    run: String,
    #[arg(long = "goal-id")]
    goal: String,
    #[arg(long = "request-id")]
    request: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ListArgs {
    #[arg(long)]
    conversation_id: String,
    #[arg(long)]
    request_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct TransitionArgs {
    #[arg(long)]
    conversation_id: String,
    #[arg(long)]
    run_id: String,
    #[arg(long)]
    goal_id: String,
    #[arg(long)]
    expected_revision: u64,
    #[arg(long, value_enum)]
    status: StatusArgument,
    #[arg(long)]
    request_id: Option<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum StatusArgument {
    Completed,
    Abandoned,
    Failed,
}

/// Exchanges one goal command after strict capability negotiation.
pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: GoalCommand,
) -> Result<GoalFrame, Box<dyn std::error::Error>> {
    match command {
        GoalCommand::Transition(args) => transition(data_dir, no_autostart, args).await,
        other => exchange(data_dir, no_autostart, frame(other)).await,
    }
}

/// Creates a shorthand goal only after the caller supplied an existing run binding.
pub(crate) async fn create_shorthand(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    run_id: String,
    summary: String,
) -> Result<GoalRecord, Box<dyn std::error::Error>> {
    let request = GoalFrame::Create {
        request_id: request_id(None),
        goal: active_goal(conversation_id, run_id, summary, None),
    };
    let GoalFrame::Created { goal, .. } = exchange(data_dir, no_autostart, request).await? else {
        return Err("daemon did not create the requested goal".into());
    };
    Ok(goal)
}

async fn transition(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: TransitionArgs,
) -> Result<GoalFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(&capabilities)?;
    write_frame(&mut stream, &WireFrame::StatusRequest).await?;
    let WireFrame::Status(status) = read_wire_or_error(&mut stream).await? else {
        return Err("daemon did not return host status before goal transition".into());
    };
    exchange_stream(
        &mut stream,
        GoalFrame::Transition {
            request_id: request_id(args.request_id),
            transition: GoalTransition {
                binding: binding(args.conversation_id, args.run_id, args.goal_id),
                expected_revision: args.expected_revision,
                host_epoch: status.host_epoch,
                next_status: args.status.into(),
            },
        },
    )
    .await
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: GoalFrame,
) -> Result<GoalFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(&capabilities)?;
    exchange_stream(&mut stream, request).await
}

async fn exchange_stream(
    stream: &mut LocalStream,
    request: GoalFrame,
) -> Result<GoalFrame, Box<dyn std::error::Error>> {
    request.validate()?;
    write_json_frame(stream, &request).await?;
    let raw: Value = read_json_frame(stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    let response: GoalFrame =
        serde_json::from_value(raw).map_err(|_| "daemon did not return a goal response")?;
    response.validate()?;
    valid_reply(&request, &response)
        .then_some(response)
        .ok_or_else(|| "daemon returned a goal response with different identity".into())
}

fn frame(command: GoalCommand) -> GoalFrame {
    match command {
        GoalCommand::Create(args) => GoalFrame::Create {
            request_id: request_id(args.request_id),
            goal: active_goal(
                args.conversation_id,
                args.run_id,
                args.summary,
                args.goal_id,
            ),
        },
        GoalCommand::Read(args) => GoalFrame::Read {
            request_id: request_id(args.request),
            binding: binding(args.conversation, args.run, args.goal),
        },
        GoalCommand::List(args) => GoalFrame::List {
            request_id: request_id(args.request_id),
            conversation_id: AgentChatConversationId(args.conversation_id),
        },
        GoalCommand::Transition(_) => unreachable!("goal transitions require a fresh host epoch"),
    }
}

fn active_goal(
    conversation_id: String,
    run_id: String,
    summary: String,
    goal_id: Option<String>,
) -> GoalRecord {
    GoalRecord {
        schema_version: GOAL_SCHEMA_VERSION,
        binding: binding(
            conversation_id,
            run_id,
            goal_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        ),
        revision: 1,
        status: GoalStatus::Active,
        summary,
    }
}

fn binding(conversation_id: String, run_id: String, goal_id: String) -> GoalBinding {
    GoalBinding {
        goal_id,
        conversation_id: AgentChatConversationId(conversation_id),
        run_id: AgentChatRunId(run_id),
    }
}

fn request_id(value: Option<String>) -> String {
    value.unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

fn require_capability(
    capabilities: &gent_types::CapabilitySet,
) -> Result<(), Box<dyn std::error::Error>> {
    capabilities
        .0
        .iter()
        .any(|item| item == GOAL_CAPABILITY)
        .then_some(())
        .ok_or_else(|| {
            "goal capability is unavailable while gentd runs in observer mode; no provider work was started".into()
        })
}

async fn read_wire_or_error(
    stream: &mut LocalStream,
) -> Result<WireFrame, Box<dyn std::error::Error>> {
    let response = gent_protocol::read_frame(stream).await?;
    if let WireFrame::Error { message, .. } = &response {
        return Err(message.clone().into());
    }
    Ok(response)
}

#[cfg(all(test, unix))]
#[path = "goal_cli_tests.rs"]
mod tests;
