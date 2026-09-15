use crate::chat_cli::Provider;
use crate::cli_error::{CliError, Failure};
use crate::local_ipc::connect_and_negotiate;
use clap::{Args, Subcommand, ValueEnum};
use gent_protocol::{
    REVIEWED_PLAN_CAPABILITY, ReviewedPlanFrame, WireFrame, read_json_frame, write_frame,
    write_json_frame,
};
use gent_types::AgentChatEffort;
use gent_types::{
    AgentChatConversationId, AgentChatMode, AgentChatRequestId, AgentChatRunId, AgentChatSelection,
    ContextPolicy, PlanArtifact, PlanStatus, ReceiptId, ReviewedPlanId, StartImplementationRequest,
};
use serde_json::Value;
use std::path::PathBuf;
#[path = "reviewed_plan_cli_conversions.rs"]
mod conversions;

#[derive(Debug, Subcommand)]
pub(crate) enum ReviewedPlanCommand {
    #[command(about = "Print the current plan proposed in a plan-mode conversation")]
    Review(ReviewArgs),
    #[command(about = "Approve the current plan and start implementing it in agent mode")]
    Start(StartArgs),
    #[command(about = "Reject the current plan so it can never be started")]
    Reject(RejectArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ReviewArgs {
    #[arg(long, help = "Conversation whose plan to print")]
    pub(crate) conversation_id: String,
    #[arg(
        long,
        help = "A specific plan; defaults to the conversation's current plan"
    )]
    pub(crate) plan_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct StartArgs {
    #[arg(long, help = "Conversation whose current plan to approve")]
    pub(crate) conversation_id: String,
    #[arg(
        long,
        value_enum,
        default_value_t = ContextArgument::Preserve,
        help = "Whether the implementation keeps the planning conversation as context"
    )]
    pub(crate) context: ContextArgument,
    #[arg(
        long,
        value_enum,
        help = "Implement with another provider; requires --model"
    )]
    pub(crate) provider: Option<Provider>,
    #[arg(
        long,
        help = "Implement with another model; defaults to the planning model"
    )]
    pub(crate) model: Option<String>,
    #[arg(
        long,
        value_parser = crate::chat_cli::parse_effort,
        help = "Implement with another effort; defaults to the planning effort"
    )]
    pub(crate) effort: Option<AgentChatEffort>,
    #[arg(long, help = "Receipt id to reuse when retrying the same approval")]
    pub(crate) receipt_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct RejectArgs {
    #[arg(long, help = "Conversation whose current plan to reject")]
    pub(crate) conversation_id: String,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum ContextArgument {
    #[default]
    Preserve,
    Clear,
}

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    command: ReviewedPlanCommand,
) -> Result<ReviewedPlanFrame, Box<dyn std::error::Error>> {
    match command {
        ReviewedPlanCommand::Review(args) => {
            exchange(
                data_dir,
                no_autostart,
                review_frame(args.conversation_id, args.plan_id.map(ReviewedPlanId)),
            )
            .await
        }
        ReviewedPlanCommand::Start(args) => start(data_dir, no_autostart, args).await,
        ReviewedPlanCommand::Reject(args) => {
            let plan = reviewable(data_dir.clone(), no_autostart, &args.conversation_id).await?;
            exchange(
                data_dir,
                no_autostart,
                ReviewedPlanFrame::Reject {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    plan_id: plan.plan_id,
                    plan_revision: plan.revision,
                    plan_content_digest_sha256: plan.content_digest_sha256,
                },
            )
            .await
        }
    }
}

async fn start(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    args: StartArgs,
) -> Result<ReviewedPlanFrame, Box<dyn std::error::Error>> {
    let plan = reviewable(data_dir.clone(), no_autostart, &args.conversation_id).await?;
    let summary =
        crate::chat_cli::summary(data_dir.clone(), no_autostart, args.conversation_id.clone())
            .await?;
    let workspace_id = summary
        .workspace_id
        .clone()
        .ok_or("the conversation has no workspace permission policy")?;
    let policy =
        crate::permissions_cli::current_for(data_dir.clone(), no_autostart, workspace_id.clone())
            .await?
            .ok_or("the conversation workspace has no permission policy")?;
    let selection = conversions::implementation_selection(
        &summary.selection,
        args.provider,
        args.model,
        args.effort,
    )?;
    let (mut stream, capabilities) = connect_and_negotiate(data_dir, no_autostart).await?;
    require_capability(&capabilities)?;
    write_frame(&mut stream, &WireFrame::StatusRequest).await?;
    let WireFrame::Status(status) = gent_protocol::read_frame(&mut stream).await? else {
        return Err("daemon did not return host status before plan approval".into());
    };
    let receipt_id = args.receipt_id.map_or_else(ReceiptId::new, ReceiptId);
    let request = StartImplementationRequest {
        request_id: AgentChatRequestId(uuid::Uuid::new_v4().to_string()),
        idempotency_key: format!("plan-start-{}", receipt_id.0),
        receipt_id,
        host_epoch: status.host_epoch,
        policy_workspace_id: workspace_id,
        policy_revision: policy.revision,
        conversation_id: AgentChatConversationId(args.conversation_id),
        plan_id: plan.plan_id,
        plan_revision: plan.revision,
        plan_content_digest_sha256: plan.content_digest_sha256,
        parent_run_id: AgentChatRunId(plan.source_run_id.0),
        selection: AgentChatSelection {
            mode: AgentChatMode::Agent,
            ..selection
        },
        context_policy: match args.context {
            ContextArgument::Preserve => ContextPolicy::Preserve,
            ContextArgument::Clear => ContextPolicy::Clear,
        },
    };
    exchange_stream(
        &mut stream,
        ReviewedPlanFrame::StartImplementation { request },
    )
    .await
}

async fn reviewable(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: &str,
) -> Result<PlanArtifact, Box<dyn std::error::Error>> {
    match exchange(
        data_dir,
        no_autostart,
        review_frame(conversation_id.into(), None),
    )
    .await?
    {
        ReviewedPlanFrame::Review {
            plan: Some(plan), ..
        } if plan.status == PlanStatus::ReadyForReview => Ok(plan),
        ReviewedPlanFrame::Review {
            plan: Some(plan), ..
        } => Err(CliError::new(
            Failure::Rejected,
            format!(
                "the current plan is {:?}, not ready for review",
                plan.status
            ),
        )
        .into()),
        _ => Err(CliError::new(Failure::NotFound, "the conversation has no plan to review").into()),
    }
}

fn review_frame(conversation_id: String, plan_id: Option<ReviewedPlanId>) -> ReviewedPlanFrame {
    ReviewedPlanFrame::ReviewRead {
        request_id: uuid::Uuid::new_v4().to_string(),
        conversation_id: AgentChatConversationId(conversation_id),
        plan_id,
    }
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
    if let Some(error) = CliError::from_reply(&raw) {
        return Err(error.into());
    }
    let response = serde_json::from_value(raw)
        .map_err(|_| "daemon did not return a reviewed-plan response")?;
    conversions::valid_reply(&request, &response)
        .then_some(response)
        .ok_or_else(|| "daemon returned a reviewed-plan response with different identity".into())
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

#[cfg(all(test, unix))]
#[path = "reviewed_plan_cli_tests.rs"]
mod tests;
