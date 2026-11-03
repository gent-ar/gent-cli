use super::{identity, journal};
use crate::runtime_activation::{
    self as authority,
    lease::{ActivationLease, LeaseFailure},
};
use gent_protocol::runtime_activation::{
    ActivationInspectionState as Observed, ActivationLookup,
    ActivationOperationInspection as Inspection, ActivationOperationRecord as Record,
    ActivationOperationState as State, ActivationOperationStatus as Status,
};
use std::path::{Path, PathBuf};

fn lookup(value: &ActivationLookup) -> Result<Record, String> {
    identity::operation_id(&value.operation_id)?;
    if !identity::valid_digest(&value.command_sha256) {
        return Err("invalid command fingerprint".into());
    }
    let root = plain_directory(&value.runtime_root)?;
    let data = plain_directory(&value.data_directory)?;
    let record = journal::read(&root, &value.operation_id)?;
    if record.command_sha256 != value.command_sha256
        || record.command.runtime_root != root
        || record.command.data_directory != data
    {
        return Err("operation lookup does not match its exact durable identity".into());
    }
    Ok(record)
}
pub(super) fn plain_directory(path: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(authority::display)?;
    if !path.is_absolute() || !metadata.is_dir() || authority::files::redirected(&metadata) {
        return Err("activation lookup directory is redirected".into());
    }
    path.canonicalize().map_err(authority::display)
}
fn status(operation: Record, state: Observed) -> Status {
    Status {
        schema: 1,
        operation,
        inspection: Inspection {
            state,
            observed_host_epoch: None,
            observed_closed_ingress: None,
            selected_release: None,
            automatic_update_helper_verified: None,
            adoption_eligible: false,
        },
    }
}
pub(super) fn inspect(value: &ActivationLookup) -> Result<Status, String> {
    let record = lookup(value)?;
    let held =
        match ActivationLease::existing_checked(&value.runtime_root, &value.data_directory, None) {
            Ok(held) => held,
            Err(LeaseFailure::Busy) => return Ok(status(record, Observed::Busy)),
            Err(_) => return Ok(status(record, Observed::Unavailable)),
        };
    inspect_held(fenced_record(value, &held)?, &held)
}
pub(super) fn inspect_record(record: &Record) -> Result<Status, String> {
    inspect(&ActivationLookup {
        operation_id: record.operation_id.clone(),
        command_sha256: record.command_sha256.clone(),
        runtime_root: record.command.runtime_root.clone(),
        data_directory: record.command.data_directory.clone(),
    })
}
fn fenced_record(value: &ActivationLookup, held: &ActivationLease) -> Result<Record, String> {
    let record = journal::read(&held.root, &value.operation_id)?;
    if record.command_sha256 != value.command_sha256
        || record.command.runtime_root != held.root
        || record.command.data_directory != held.data_dir
    {
        return Err("operation identity changed before lease acquisition".into());
    }
    Ok(record)
}
pub(super) fn inspect_held(record: Record, held: &ActivationLease) -> Result<Status, String> {
    if record.command.runtime_root != held.root || record.command.data_directory != held.data_dir {
        return Err("operation is not bound to the held runtime and data leases".into());
    }
    let mut status = status(record, Observed::Unavailable);
    status.inspection.observed_host_epoch = held.origin_host_epoch;
    status.inspection.observed_closed_ingress = held
        .origin_ingress
        .map(|i| i == gent_ports::IngressMode::Closed);
    if held.origin_host_epoch != Some(status.operation.origin_host_epoch) {
        status.inspection.state = Observed::EpochMismatch;
        return Ok(status);
    }
    if status.inspection.observed_closed_ingress != Some(true) {
        status.inspection.state = Observed::NotClosed;
        return Ok(status);
    }
    let Ok(selected) = authority::selected_release(&held.root) else {
        return Ok(status);
    };
    status.inspection.selected_release.clone_from(&selected);
    if selected.as_deref() != Some(&status.operation.selected_release) {
        status.inspection.state = Observed::SelectionMismatch;
        return Ok(status);
    }
    let Ok(actual) = authority::result(
        &held.root,
        held,
        status.operation.selected_release.clone(),
        status.operation.command.expected_daemon_sha256.as_deref(),
    ) else {
        return Ok(status);
    };
    if Some(actual.bootstrap_tree_sha256.as_str())
        != status
            .operation
            .command
            .expected_bootstrap_tree_sha256
            .as_deref()
    {
        return Ok(status);
    }
    status.inspection.automatic_update_helper_verified = Some(helper_verified(
        &held.root,
        &status.operation.selected_release,
    ));
    match status.operation.state {
        State::Completed | State::RecoveredPublication => {
            if status.operation.result.as_ref() != Some(&actual) {
                return Ok(status);
            }
            status.inspection.state = Observed::SelectionVerified;
        }
        State::PublicationIntent | State::Uncertain => {
            status.inspection.state = Observed::PublishedRecoveryRequired;
            status.inspection.adoption_eligible = true;
        }
        _ => {
            status.inspection.state = Observed::SelectionVerified;
        }
    }
    Ok(status)
}
pub(super) fn helper_verified(root: &Path, selected: &str) -> bool {
    let name = authority::scheduling::auto_update_helper_name();
    let selected = authority::stage::file_digest(&root.join("releases").join(selected).join(name));
    let active = authority::stage::file_digest(&root.join(name));
    matches!((selected, active), (Ok(left), Ok(right)) if left == right)
}
pub(super) fn adopt(value: &ActivationLookup) -> Result<Status, String> {
    let record = lookup(value)?;
    let held = ActivationLease::existing(
        &value.runtime_root,
        &value.data_directory,
        Some(record.origin_host_epoch),
    )?;
    let mut status = inspect_held(fenced_record(value, &held)?, &held)?;
    if matches!(
        status.operation.state,
        State::Completed | State::RecoveredPublication
    ) && status.inspection.state == Observed::SelectionVerified
    {
        return Ok(status);
    }
    if status.inspection.state != Observed::PublishedRecoveryRequired
        || !status.inspection.adoption_eligible
    {
        return Err("operation cannot be adopted from its current state".into());
    }
    status.operation.result = Some(authority::result(
        &held.root,
        &held,
        status.operation.selected_release.clone(),
        status.operation.command.expected_daemon_sha256.as_deref(),
    )?);
    status.operation.automatic_update_helper_verified = status
        .inspection
        .automatic_update_helper_verified
        .unwrap_or(false);
    status.operation.state = State::RecoveredPublication;
    journal::write(&status.operation, false)?;
    status.inspection.state = Observed::SelectionVerified;
    status.inspection.adoption_eligible = false;
    Ok(status)
}
