use crate::cli_error::{CliError, Failure};
use crate::command_execution::print;
use crate::local_ipc::request;
use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyActionResult, DependencyActionState,
    DependencyPlanRequest, DependencyProvider, WireFrame,
};
use gent_types::ReceiptId;

pub(crate) fn dependency_plan_frame(
    provider: DependencyProvider,
    action: DependencyAction,
) -> WireFrame {
    WireFrame::DependencyPlanRequest(DependencyPlanRequest { provider, action })
}

pub(crate) async fn dependency_action(
    data_dir: Option<std::path::PathBuf>,
    no_autostart: bool,
    provider: DependencyProvider,
    action: DependencyAction,
    consent_granted: bool,
    idempotency_key: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let plan = request(
        data_dir.clone(),
        no_autostart,
        dependency_plan_frame(provider, action),
    )
    .await?;
    let WireFrame::DependencyPlan(plan) = plan else {
        return Err("daemon did not return a dependency plan".into());
    };
    let status = request(data_dir.clone(), no_autostart, WireFrame::StatusRequest).await?;
    let WireFrame::Status(status) = status else {
        return Err("daemon did not return host status".into());
    };
    let reply = request(
        data_dir,
        no_autostart,
        WireFrame::DependencyActionRequest(DependencyActionRequest {
            provider,
            action,
            consent_granted,
            receipt_id: ReceiptId::new(),
            idempotency_key: idempotency_key.unwrap_or_else(|| ReceiptId::new().0),
            host_epoch: status.host_epoch,
            reviewed_plan_digest: plan.reviewed_plan_digest,
        }),
    )
    .await?;
    print(&reply)?;
    let WireFrame::DependencyActionResult(result) = reply else {
        return Err("daemon did not return a dependency action result".into());
    };
    action_outcome(&result)
}

fn action_outcome(result: &DependencyActionResult) -> Result<(), Box<dyn std::error::Error>> {
    let verb = match result.plan.action {
        DependencyAction::Install => "install",
        DependencyAction::Update => "update",
    };
    let failure = match result.state {
        DependencyActionState::Completed => return Ok(()),
        DependencyActionState::ConsentRequired => CliError::new(
            Failure::ConsentRequired,
            format!(
                "nothing was changed; review the plan above and rerun with --consent to {verb}"
            ),
        ),
        DependencyActionState::PlanMismatch => CliError::new(
            Failure::Rejected,
            "the plan changed while it was being reviewed; nothing was changed, review it again",
        ),
        DependencyActionState::Failed | DependencyActionState::Unprovable => CliError::new(
            Failure::Rejected,
            result.detail.as_ref().map_or_else(
                || format!("the {verb} did not complete"),
                |detail| format!("the {verb} did not complete: {detail}"),
            ),
        ),
    };
    Err(failure.into())
}

pub(crate) async fn dependency(
    data_dir: Option<std::path::PathBuf>,
    no_autostart: bool,
    command: crate::DependencyCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        crate::DependencyCommand::Plan { action, provider } => print(
            request(
                data_dir,
                no_autostart,
                dependency_plan_frame(provider, action),
            )
            .await?,
        ),
        crate::DependencyCommand::Install {
            provider,
            consent,
            idempotency_key,
        } => {
            dependency_action(
                data_dir,
                no_autostart,
                provider,
                DependencyAction::Install,
                consent,
                idempotency_key,
            )
            .await
        }
        crate::DependencyCommand::Update {
            provider,
            consent,
            idempotency_key,
        } => {
            dependency_action(
                data_dir,
                no_autostart,
                provider,
                DependencyAction::Update,
                consent,
                idempotency_key,
            )
            .await
        }
    }
}
