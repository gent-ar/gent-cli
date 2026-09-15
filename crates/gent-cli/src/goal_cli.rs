//! Capability-gated terminal commands for durable, provider-neutral `/goal` records.
//!
//! This client only exchanges typed goal frames with local gentd. It never starts a
//! provider, derives a goal from provider output, or changes daemon authority.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use gent_protocol::{GOAL_CAPABILITY, GoalFrame, read_json_frame, write_json_frame};
use gent_types::{AgentChatConversationId, GoalRecord, GoalReportOutcome};
use serde_json::Value;

use crate::local_ipc::{LocalStream, connect_and_negotiate};

mod reply;
use reply::valid_reply;

#[derive(Debug, Subcommand)]
pub(crate) enum GoalCommand {
    #[command(about = "Set the goal Gent keeps working on until it is complete or blocked")]
    Set(SetArgs),
    #[command(about = "Pause work on the goal")]
    Pause(ControlArgs),
    #[command(about = "Resume work on a paused goal")]
    Resume(ControlArgs),
    #[command(about = "Remove the goal")]
    Clear(ControlArgs),
    #[command(about = "Show the conversation's goal as JSON")]
    Show(ShowArgs),
}

#[derive(Debug, Args)]
pub(crate) struct SetArgs {
    #[arg(long, help = "Conversation that pursues the goal")]
    conversation_id: String,
    #[arg(value_name = "OBJECTIVE", help = "What the goal should achieve")]
    objective: String,
    #[arg(long, help = "Stop pursuing the goal after this many tokens")]
    token_budget: Option<u64>,
    #[arg(long, help = "Client request id used to correlate the reply")]
    request_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ControlArgs {
    #[arg(long, help = "Conversation whose goal changes")]
    conversation_id: String,
    #[arg(
        long,
        help = "Apply only if the goal is still at this revision [default: current revision]"
    )]
    expected_revision: Option<u64>,
    #[arg(long, help = "Client request id used to correlate the reply")]
    request_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ShowArgs {
    #[arg(long, help = "Conversation whose goal is shown")]
    conversation_id: String,
    #[arg(long, help = "Client request id used to correlate the reply")]
    request_id: Option<String>,
}

#[derive(Clone, Copy, Debug)]
enum Control {
    Pause,
    Resume,
    Clear,
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: GoalCommand,
) -> Result<GoalFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(&capabilities)?;
    match command {
        GoalCommand::Set(args) => {
            exchange(
                &mut stream,
                GoalFrame::Set {
                    request_id: request_id(args.request_id),
                    conversation_id: AgentChatConversationId(args.conversation_id),
                    objective: args.objective,
                    token_budget: args.token_budget,
                },
            )
            .await
        }
        GoalCommand::Pause(args) => control(&mut stream, args, Control::Pause).await,
        GoalCommand::Resume(args) => control(&mut stream, args, Control::Resume).await,
        GoalCommand::Clear(args) => control(&mut stream, args, Control::Clear).await,
        GoalCommand::Show(args) => {
            exchange(
                &mut stream,
                GoalFrame::Read {
                    request_id: request_id(args.request_id),
                    conversation_id: AgentChatConversationId(args.conversation_id),
                },
            )
            .await
        }
    }
}

pub(crate) async fn report(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    goal_id: String,
    outcome: GoalReportOutcome,
    note: Option<String>,
) -> Result<GoalRecord, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(&capabilities)?;
    let reply = exchange(
        &mut stream,
        GoalFrame::Report {
            request_id: request_id(None),
            goal_id,
            outcome,
            note,
        },
    )
    .await?;
    let GoalFrame::Goal {
        goal: Some(goal), ..
    } = reply
    else {
        return Err("daemon did not settle the reported goal".into());
    };
    Ok(goal)
}

async fn control(
    stream: &mut LocalStream,
    args: ControlArgs,
    control: Control,
) -> Result<GoalFrame, Box<dyn std::error::Error>> {
    let conversation_id = AgentChatConversationId(args.conversation_id);
    let GoalFrame::Goal {
        goal: Some(goal), ..
    } = exchange(
        stream,
        GoalFrame::Read {
            request_id: request_id(None),
            conversation_id: conversation_id.clone(),
        },
    )
    .await?
    else {
        return Err("conversation has no goal; nothing was changed".into());
    };
    let request_id = request_id(args.request_id);
    let goal_id = goal.binding.goal_id;
    let expected_revision = args.expected_revision.unwrap_or(goal.revision);
    let frame = match control {
        Control::Pause => GoalFrame::Pause {
            request_id,
            conversation_id,
            goal_id,
            expected_revision,
        },
        Control::Resume => GoalFrame::Resume {
            request_id,
            conversation_id,
            goal_id,
            expected_revision,
        },
        Control::Clear => GoalFrame::Clear {
            request_id,
            conversation_id,
            goal_id,
            expected_revision,
        },
    };
    exchange(stream, frame).await
}

async fn exchange(
    stream: &mut LocalStream,
    request: GoalFrame,
) -> Result<GoalFrame, Box<dyn std::error::Error>> {
    request.validate()?;
    write_json_frame(stream, &request).await?;
    let raw: Value = read_json_frame(stream).await?;
    if let Some(error) = crate::cli_error::CliError::from_reply(&raw) {
        return Err(error.into());
    }
    let response: GoalFrame =
        serde_json::from_value(raw).map_err(|_| "daemon did not return a goal response")?;
    response.validate()?;
    if !valid_reply(&request, &response) {
        return Err("daemon returned a goal response with different identity".into());
    }
    if let GoalFrame::Rejected { code, .. } = response {
        return Err(format!(
            "goal request was rejected: {}",
            serde_json::to_value(code)?.as_str().unwrap_or_default()
        )
        .into());
    }
    Ok(response)
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

#[cfg(all(test, unix))]
#[path = "goal_cli_tests.rs"]
mod tests;
