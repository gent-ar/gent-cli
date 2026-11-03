use gent_protocol::runtime_activation::{
    ACTIVATION_OPERATION_CAPABILITY, ActivationMaintenanceAction as Action,
    ActivationMaintenanceReply as Reply,
};
use std::path::Path;
#[path = "runtime_activation_operation_identity.rs"]
mod identity;
#[path = "runtime_activation_operation_journal.rs"]
mod journal;
#[path = "runtime_activation_origin.rs"]
mod origin;
#[path = "runtime_activation_operation_plan.rs"]
mod plan;
#[path = "runtime_activation_operation_status.rs"]
mod status;
#[path = "runtime_activation_operation_workflow.rs"]
mod workflow;

pub(crate) fn self_digest(path: &Path) -> Result<String, String> {
    super::stage::file_digest(path)
}

pub(crate) fn dispatch(action: Action) -> Result<Reply, String> {
    Ok(match action {
        Action::Capabilities => Reply::Capabilities {
            schema: 1,
            capabilities: vec![
                "runtime-activation-lease-v1".into(),
                "runtime-immutable-release-v1".into(),
                ACTIVATION_OPERATION_CAPABILITY.into(),
                "runtime-maintenance-pinned-delegation-v1".into(),
                "runtime-activation-origin-status-v1".into(),
            ],
        },
        Action::BootstrapIdentity {
            bootstrap_directory,
        } => Reply::BootstrapIdentity(super::bootstrap_identity(&bootstrap_directory)?),
        Action::Plan(command) => Reply::Planned(Box::new(plan::prepare(command)?)),
        Action::OriginStatus(value) => Reply::Origin(origin::inspect(&value)),
        Action::Activate(command) if command.operation.is_some() => {
            let record = workflow::activate(command, |_| Ok(()))?;
            Reply::Operation(Box::new(status::inspect_record(&record)?))
        }
        Action::Activate(command) => {
            let scheduler = if command.enable_automatic_updates {
                super::enable_updates
            } else {
                no_schedule
            };
            Reply::Activated(super::activate_at_expected(
                &command.bootstrap_directory,
                &command.runtime_root,
                &command.data_directory,
                scheduler,
                command.expected_daemon_sha256.as_deref(),
                command.expected_host_epoch,
                command.expected_bootstrap_tree_sha256.as_deref(),
            )?)
        }
        Action::Status(lookup) => Reply::Operation(Box::new(status::inspect(&lookup)?)),
        Action::Adopt(lookup) => Reply::Operation(Box::new(status::adopt(&lookup)?)),
    })
}
fn no_schedule(_: &Path, _: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
#[path = "runtime_activation_operation_tests.rs"]
mod tests;
