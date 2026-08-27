use crate::command_execution::print;
use crate::local_ipc::request;
use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyPlanRequest, DependencyProvider, WireFrame,
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
    print(
        request(
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
        .await?,
    )
}
