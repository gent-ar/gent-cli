use super::identity;
use crate::runtime_activation as authority;
use gent_protocol::runtime_activation::{ActivationCommand, ActivationOperationPlan};

pub(super) fn prepare(mut command: ActivationCommand) -> Result<ActivationOperationPlan, String> {
    let op = command
        .operation
        .as_ref()
        .ok_or("planning requires an operation UUID")?;
    identity::operation_id(&op.operation_id)?;
    if command.expected_host_epoch.is_none_or(|epoch| epoch == 0)
        || !command
            .expected_daemon_sha256
            .as_deref()
            .is_some_and(identity::valid_digest)
        || !command
            .expected_bootstrap_tree_sha256
            .as_deref()
            .is_some_and(identity::valid_digest)
        || !identity::valid_digest(&command.maintenance_daemon_sha256)
        || op.expected_owner.process_id == 0
        || !identity::valid_digest(&op.expected_owner.executable_digest_sha256)
    {
        return Err("planning requires exact epoch, tree, digest and expected owner".into());
    }
    for path in [
        &mut command.bootstrap_directory,
        &mut command.runtime_root,
        &mut command.data_directory,
    ] {
        if !path.is_absolute() {
            return Err("planning requires absolute directories".into());
        }
        let metadata = std::fs::symlink_metadata(&*path).map_err(authority::display)?;
        if !metadata.is_dir() || authority::files::redirected(&metadata) {
            return Err("planning directory is redirected".into());
        }
        *path = path.canonicalize().map_err(authority::display)?;
    }
    if command
        .runtime_root
        .starts_with(&command.bootstrap_directory)
        || command
            .data_directory
            .starts_with(&command.bootstrap_directory)
    {
        return Err("runtime store and owner data cannot be inside bootstrap source".into());
    }
    let actual = authority::bootstrap_identity(&command.bootstrap_directory)?;
    if Some(actual.executable_digest_sha256.as_str()) != command.expected_daemon_sha256.as_deref()
        || Some(actual.bootstrap_tree_sha256.as_str())
            != command.expected_bootstrap_tree_sha256.as_deref()
    {
        return Err("bootstrap differs from reviewed identity".into());
    }
    let descriptor: authority::Bootstrap = serde_json::from_slice(
        &std::fs::read(command.bootstrap_directory.join("bootstrap.json"))
            .map_err(authority::display)?,
    )
    .map_err(authority::display)?;
    let platform = authority::expected_target()?;
    let command_sha256 = identity::fingerprint(&command, &platform, &descriptor.version)?;
    if command
        .expected_command_sha256
        .as_deref()
        .is_some_and(|value| value != command_sha256)
    {
        return Err("operation plan identity changed".into());
    }
    command.expected_command_sha256 = Some(command_sha256.clone());
    let selected_release = format!(
        "{}+{}.{}-{}",
        descriptor.version,
        &actual.bootstrap_tree_sha256[..16],
        command_sha256,
        platform
    );
    Ok(ActivationOperationPlan {
        schema: 1,
        operation_id: command
            .operation
            .as_ref()
            .ok_or("operation missing")?
            .operation_id
            .clone(),
        command_sha256,
        command,
        platform,
        version: descriptor.version,
        selected_release,
    })
}
