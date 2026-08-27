//! Bounded terminal requests for Gent-owned, provider-neutral task graphs.
//!
//! The JSON files are strict `gent-types` request values. This client only forwards them after
//! local capability negotiation; it does not select commands, launch workers, or parse provider
//! output.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use gent_protocol::{
    ORCHESTRATION_CAPABILITY, OrchestrationFrame, WireFrame, read_json_frame, write_json_frame,
};
use gent_types::AgentChatConversationId;
use serde_json::Value;

use crate::local_ipc::{LocalStream, connect_and_negotiate};

mod reply;
use reply::valid_reply;

/// Terminal actions on a daemon-owned task graph.
#[derive(Debug, Subcommand)]
pub(crate) enum OrchestrationCommand {
    /// Submit an exact `FanoutRequest` JSON value from a local development file.
    Fanout(FanoutArgs),
    /// Submit an exact `CrossReviewRequest` JSON value from a local development file.
    CrossReview(CrossReviewArgs),
    /// Read one graph without exposing provider-local work state.
    Read(ReadArgs),
}

#[derive(Debug, Args)]
pub(crate) struct FanoutArgs {
    /// Path to a strict JSON-encoded `FanoutRequest`.
    #[arg(long)]
    graph_json: PathBuf,
    #[arg(long)]
    request_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct CrossReviewArgs {
    /// Path to a strict JSON-encoded `CrossReviewRequest`.
    #[arg(long)]
    request_json: PathBuf,
    #[arg(long)]
    request_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ReadArgs {
    #[arg(long = "conversation-id")]
    conversation: String,
    #[arg(long = "graph-id")]
    graph: String,
    #[arg(long = "request-id")]
    request: Option<String>,
}

/// Exchanges one finite graph request after the daemon explicitly exposes its authority.
pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: OrchestrationCommand,
) -> Result<OrchestrationFrame, Box<dyn std::error::Error>> {
    exchange(data_dir, no_autostart, frame(command)?).await
}

/// Forwards one exact shorthand fanout file without introducing prompt semantics.
pub(crate) async fn fanout_file(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    path: PathBuf,
) -> Result<OrchestrationFrame, Box<dyn std::error::Error>> {
    exchange(data_dir, no_autostart, fanout_frame(&path, None)?).await
}

/// Forwards one exact shorthand cross-review file without introducing prompt semantics.
pub(crate) async fn cross_review_file(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    path: PathBuf,
) -> Result<OrchestrationFrame, Box<dyn std::error::Error>> {
    exchange(data_dir, no_autostart, cross_review_frame(&path, None)?).await
}

fn frame(command: OrchestrationCommand) -> Result<OrchestrationFrame, Box<dyn std::error::Error>> {
    match command {
        OrchestrationCommand::Fanout(args) => fanout_frame(&args.graph_json, args.request_id),
        OrchestrationCommand::CrossReview(args) => {
            cross_review_frame(&args.request_json, args.request_id)
        }
        OrchestrationCommand::Read(args) => Ok(OrchestrationFrame::GraphRead {
            request_id: request_id(args.request),
            conversation_id: AgentChatConversationId(args.conversation),
            graph_id: args.graph,
        }),
    }
}

fn fanout_frame(
    path: &Path,
    request_id_value: Option<String>,
) -> Result<OrchestrationFrame, Box<dyn std::error::Error>> {
    Ok(OrchestrationFrame::Fanout {
        request_id: request_id(request_id_value),
        request: read_request(path, "graph", "FanoutRequest")?,
    })
}

fn cross_review_frame(
    path: &Path,
    request_id_value: Option<String>,
) -> Result<OrchestrationFrame, Box<dyn std::error::Error>> {
    Ok(OrchestrationFrame::CrossReview {
        request_id: request_id(request_id_value),
        request: read_request(path, "cross-review", "CrossReviewRequest")?,
    })
}

fn read_request<T>(
    path: &Path,
    name: &str,
    type_name: &str,
) -> Result<T, Box<dyn std::error::Error>>
where
    T: serde::de::DeserializeOwned,
{
    let input =
        std::fs::read_to_string(path).map_err(|_| format!("could not read {name} JSON input"))?;
    serde_json::from_str(&input)
        .map_err(|_| format!("{name} JSON must be one exact {type_name} value").into())
}

async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: OrchestrationFrame,
) -> Result<OrchestrationFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(&capabilities)?;
    exchange_stream(&mut stream, request).await
}

async fn exchange_stream(
    stream: &mut LocalStream,
    request: OrchestrationFrame,
) -> Result<OrchestrationFrame, Box<dyn std::error::Error>> {
    request.validate()?;
    write_json_frame(stream, &request).await?;
    let raw: Value = read_json_frame(stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    let response: OrchestrationFrame = serde_json::from_value(raw)
        .map_err(|_| "daemon did not return an orchestration response")?;
    response.validate()?;
    valid_reply(&request, &response)
        .then_some(response)
        .ok_or_else(|| "daemon returned an orchestration response with different identity".into())
}

fn require_capability(
    capabilities: &gent_types::CapabilitySet,
) -> Result<(), Box<dyn std::error::Error>> {
    capabilities
        .0
        .iter()
        .any(|item| item == ORCHESTRATION_CAPABILITY)
        .then_some(())
        .ok_or_else(|| {
            "orchestration capability is unavailable while gentd runs in observer mode; no worker was started"
                .into()
        })
}

fn request_id(value: Option<String>) -> String {
    value.unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

#[cfg(all(test, unix))]
#[path = "orchestration_cli_tests.rs"]
mod tests;
