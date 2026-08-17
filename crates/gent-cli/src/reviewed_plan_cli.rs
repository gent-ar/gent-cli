//! Terminal client for Gent-owned reviewed-plan lifecycle commands.
//!
//! Both the terminal and a native host use these capability-gated protocol values. This client
//! never evaluates a plan, mutates history, or starts a provider process.

use crate::local_ipc::connect_and_negotiate;
use clap::{Args, Subcommand, ValueEnum};
use gent_protocol::{
    REVIEWED_PLAN_CAPABILITY, ReviewedPlanFrame, WireFrame, read_json_frame, write_frame,
    write_json_frame,
};
use gent_types::{
    AgentChatConversationId, AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatRequestId,
    AgentChatRunId, AgentChatSelection, ContextPolicy, PlanRevision, ReceiptId, ReviewedPlanId,
    StartImplementationRequest,
};
use serde_json::Value;
use std::path::PathBuf;
/// The terminal-facing reviewed-plan actions also used by native-host clients.
#[derive(Debug, Subcommand)]
pub(crate) enum ReviewedPlanCommand {
    /// Read the current immutable plan artifact for a conversation.
    Review(ReviewArgs),
    /// Approve one exact plan revision and reserve its child implementation run.
    Start(StartArgs),
    /// Reject one exact immutable plan revision without mutating conversation history.
    Reject(RejectArgs),
}
#[derive(Debug, Args)]
#[allow(clippy::struct_field_names)] // These exact public CLI flags name distinct protocol identities.
pub(crate) struct ReviewArgs {
    #[arg(long)]
    conversation_id: String,
    #[arg(long)]
    plan_id: String,
    #[arg(long)]
    request_id: Option<String>,
}
#[derive(Debug, Args)]
pub(crate) struct StartArgs {
    #[arg(long)]
    conversation_id: String,
    #[arg(long)]
    plan_id: String,
    #[arg(long)]
    plan_revision: u64,
    #[arg(long)]
    plan_content_digest_sha256: String,
    #[arg(long)]
    parent_run_id: String,
    #[arg(long, value_enum)]
    provider: ProviderArgument,
    #[arg(long)]
    model: String,
    #[arg(long, value_enum, default_value_t = EffortArgument::Medium)]
    effort: EffortArgument,
    #[arg(long, value_enum, default_value_t = ModeArgument::Agent)]
    mode: ModeArgument,
    #[arg(long, value_enum, default_value_t = ContextArgument::Preserve)]
    context: ContextArgument,
    /// The revision observed from Gent's durable permission policy before approval.
    #[arg(long)]
    policy_revision: u64,
    #[arg(long)]
    request_id: Option<String>,
    #[arg(long)]
    receipt_id: Option<String>,
    #[arg(long)]
    idempotency_key: Option<String>,
}
#[derive(Debug, Args)]
pub(crate) struct RejectArgs {
    #[arg(long)]
    plan_id: String,
    #[arg(long)]
    plan_revision: u64,
    #[arg(long)]
    plan_content_digest_sha256: String,
    #[arg(long)]
    request_id: Option<String>,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ProviderArgument {
    Claude,
    Codex,
    Claurst,
}
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum EffortArgument {
    Low,
    #[default]
    Medium,
    High,
}
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum ModeArgument {
    Ask,
    Plan,
    #[default]
    Agent,
}
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum ContextArgument {
    #[default]
    Preserve,
    Clear,
}
/// Executes one reviewed-plan request without introducing terminal-specific lifecycle logic.
pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: ReviewedPlanCommand,
) -> Result<ReviewedPlanFrame, Box<dyn std::error::Error>> {
    match command {
        ReviewedPlanCommand::Start(args) => start(data_dir, no_autostart, args).await,
        other => exchange(data_dir, no_autostart, frame(other)).await,
    }
}
async fn start(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: StartArgs,
) -> Result<ReviewedPlanFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(&capabilities)?;
    write_frame(&mut stream, &WireFrame::StatusRequest).await?;
    let WireFrame::Status(status) = read_frame_or_error(&mut stream).await? else {
        return Err("daemon did not return host status before plan approval".into());
    };
    let frame = ReviewedPlanFrame::StartImplementation {
        request: start_request(args, status.host_epoch),
    };
    exchange_stream(&mut stream, frame).await
}
async fn exchange(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    request: ReviewedPlanFrame,
) -> Result<ReviewedPlanFrame, Box<dyn std::error::Error>> {
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(&capabilities)?;
    exchange_stream(&mut stream, request).await
}
async fn exchange_stream(
    stream: &mut crate::local_ipc::LocalStream,
    request: ReviewedPlanFrame,
) -> Result<ReviewedPlanFrame, Box<dyn std::error::Error>> {
    request.validate()?;
    write_json_frame(stream, &request).await?;
    let raw: Value = read_json_frame(stream).await?;
    if let Ok(WireFrame::Error { message, .. }) = serde_json::from_value(raw.clone()) {
        return Err(message.into());
    }
    let response = serde_json::from_value(raw)
        .map_err(|_| "daemon did not return a reviewed-plan response")?;
    valid_reply(&request, &response)
        .then_some(response)
        .ok_or_else(|| "daemon returned a reviewed-plan response with different identity".into())
}
async fn read_frame_or_error(
    stream: &mut crate::local_ipc::LocalStream,
) -> Result<WireFrame, Box<dyn std::error::Error>> {
    let response = gent_protocol::read_frame(stream).await?;
    if let WireFrame::Error { message, .. } = &response {
        return Err(message.clone().into());
    }
    Ok(response)
}

fn require_capability(
    capabilities: &gent_types::CapabilitySet,
) -> Result<(), Box<dyn std::error::Error>> {
    capabilities
        .0
        .iter()
        .any(|item| item == REVIEWED_PLAN_CAPABILITY)
        .then_some(())
        .ok_or_else(|| {
            "reviewed-plan capability is unavailable while gentd runs in observer mode; no provider work was started".into()
        })
}

fn frame(command: ReviewedPlanCommand) -> ReviewedPlanFrame {
    match command {
        ReviewedPlanCommand::Review(args) => ReviewedPlanFrame::ReviewRead {
            request_id: request_id(args.request_id),
            conversation_id: AgentChatConversationId(args.conversation_id),
            plan_id: ReviewedPlanId(args.plan_id),
        },
        ReviewedPlanCommand::Reject(args) => ReviewedPlanFrame::Reject {
            request_id: request_id(args.request_id),
            plan_id: ReviewedPlanId(args.plan_id),
            plan_revision: PlanRevision(args.plan_revision),
            plan_content_digest_sha256: args.plan_content_digest_sha256,
        },
        ReviewedPlanCommand::Start(_) => unreachable!("start requires a fresh host epoch"),
    }
}

fn start_request(args: StartArgs, host_epoch: gent_types::HostEpoch) -> StartImplementationRequest {
    StartImplementationRequest {
        request_id: AgentChatRequestId(request_id(args.request_id)),
        receipt_id: args.receipt_id.map_or_else(ReceiptId::new, ReceiptId),
        idempotency_key: args
            .idempotency_key
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        host_epoch,
        policy_workspace_id: "gent-local-settings".into(),
        policy_revision: args.policy_revision,
        conversation_id: AgentChatConversationId(args.conversation_id),
        plan_id: ReviewedPlanId(args.plan_id),
        plan_revision: PlanRevision(args.plan_revision),
        plan_content_digest_sha256: args.plan_content_digest_sha256,
        parent_run_id: AgentChatRunId(args.parent_run_id),
        selection: AgentChatSelection {
            provider: args.provider.into(),
            model: args.model,
            effort: args.effort.into(),
            mode: args.mode.into(),
        },
        context_policy: args.context.into(),
    }
}

fn request_id(value: Option<String>) -> String {
    value.unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

fn valid_reply(request: &ReviewedPlanFrame, response: &ReviewedPlanFrame) -> bool {
    match (request, response) {
        (
            ReviewedPlanFrame::ReviewRead { request_id, .. },
            ReviewedPlanFrame::Review {
                request_id: reply, ..
            },
        ) => reply == request_id,
        (
            ReviewedPlanFrame::StartImplementation { request },
            ReviewedPlanFrame::StartedImplementation { request_id, result },
        ) => request_id == &request.request_id.0 && result.receipt.receipt_id == request.receipt_id,
        (
            ReviewedPlanFrame::Reject {
                request_id,
                plan_id,
                plan_revision,
                ..
            },
            ReviewedPlanFrame::Rejected {
                request_id: reply,
                plan_id: rejected,
                plan_revision: revision,
            },
        ) => reply == request_id && rejected == plan_id && revision == plan_revision,
        _ => false,
    }
}

impl From<ProviderArgument> for AgentChatProvider {
    fn from(value: ProviderArgument) -> Self {
        match value {
            ProviderArgument::Claude => Self::Claude,
            ProviderArgument::Codex => Self::Codex,
            ProviderArgument::Claurst => Self::Claurst,
        }
    }
}

impl From<EffortArgument> for AgentChatEffort {
    fn from(value: EffortArgument) -> Self {
        match value {
            EffortArgument::Low => Self::Low,
            EffortArgument::Medium => Self::Medium,
            EffortArgument::High => Self::High,
        }
    }
}

impl From<ModeArgument> for AgentChatMode {
    fn from(value: ModeArgument) -> Self {
        match value {
            ModeArgument::Ask => Self::Ask,
            ModeArgument::Plan => Self::Plan,
            ModeArgument::Agent => Self::Agent,
        }
    }
}

impl From<ContextArgument> for ContextPolicy {
    fn from(value: ContextArgument) -> Self {
        match value {
            ContextArgument::Preserve => Self::Preserve,
            ContextArgument::Clear => Self::Clear,
        }
    }
}

#[cfg(all(test, unix))]
#[path = "reviewed_plan_cli_tests.rs"]
mod tests;
