use super::{identity, journal};
use crate::runtime_activation::{self as authority, lease::ActivationLease};
use gent_protocol::runtime_activation::{
    ActivationCommand, ActivationOperationRecord as Record, ActivationOperationState as State,
};
use std::fs;

pub(super) fn activate(
    mut command: ActivationCommand,
    checkpoint: impl Fn(&str) -> Result<(), String>,
) -> Result<Record, String> {
    let op = command
        .operation
        .as_ref()
        .ok_or("tracked operation required")?;
    identity::operation_id(&op.operation_id)?;
    if command.expected_host_epoch.is_none_or(|e| e == 0)
        || !command
            .expected_daemon_sha256
            .as_deref()
            .is_some_and(identity::valid_digest)
        || !command
            .expected_bootstrap_tree_sha256
            .as_deref()
            .is_some_and(identity::valid_digest)
        || !identity::valid_digest(&op.expected_owner.executable_digest_sha256)
        || op.expected_owner.process_id == 0
    {
        return Err(
            "tracked activation requires exact epoch, tree, digest and expected owner".into(),
        );
    }
    let held = ActivationLease::acquire(
        &command.runtime_root,
        &command.data_directory,
        command.expected_host_epoch,
    )?;
    command.runtime_root.clone_from(&held.root);
    command.data_directory.clone_from(&held.data_dir);
    command.bootstrap_directory = command
        .bootstrap_directory
        .canonicalize()
        .map_err(authority::display)?;
    if held.root.starts_with(&command.bootstrap_directory)
        || held.data_dir.starts_with(&command.bootstrap_directory)
    {
        return Err("runtime store and owner data cannot be inside the bootstrap source".into());
    }
    let descriptor: authority::Bootstrap = serde_json::from_slice(
        &fs::read(command.bootstrap_directory.join("bootstrap.json"))
            .map_err(authority::display)?,
    )
    .map_err(authority::display)?;
    let platform = authority::expected_target()?;
    if descriptor.target != platform || !authority::valid_version(&descriptor.version) {
        return Err("bootstrap metadata does not match this platform".into());
    }
    let fingerprint = identity::fingerprint(&command, &platform, &descriptor.version)?;
    if command.expected_command_sha256.as_deref() != Some(fingerprint.as_str()) {
        return Err("tracked activation differs from its reviewed operation plan".into());
    }
    let operation_id = command
        .operation
        .as_ref()
        .ok_or("operation missing")?
        .operation_id
        .clone();
    let path = held
        .root
        .join("activation-operations")
        .join(format!("{operation_id}.json"));
    if fs::symlink_metadata(&path).is_ok() {
        let existing = journal::read(&held.root, &operation_id)?;
        if existing.command_sha256 != fingerprint {
            return Err("operation ID already bound to another command".into());
        }
        return Ok(existing);
    }
    authority::verify_bootstrap(&command.bootstrap_directory)?;
    let actual = authority::bootstrap_identity(&command.bootstrap_directory)?;
    if Some(actual.bootstrap_tree_sha256.as_str())
        != command.expected_bootstrap_tree_sha256.as_deref()
        || Some(actual.executable_digest_sha256.as_str())
            != command.expected_daemon_sha256.as_deref()
    {
        return Err("prepared bootstrap changed before durable acceptance".into());
    }
    if authority::selected_release(&held.root)?
        .as_ref()
        .is_some_and(|s| authority::version_of(s) > authority::version_of(&descriptor.version))
    {
        return Err("a newer runtime was selected before activation".into());
    }
    let selected_release = format!(
        "{}+{}.{}-{}",
        descriptor.version,
        &actual.bootstrap_tree_sha256[..16],
        fingerprint,
        platform
    );
    let mut record = Record {
        schema: 1,
        operation_id,
        command_sha256: fingerprint,
        command,
        platform,
        version: descriptor.version,
        origin_host_epoch: held.origin_host_epoch.ok_or("origin epoch missing")?,
        effect_issuer_process_id: std::process::id(),
        selected_release,
        state: State::Accepted,
        automatic_update_helper_verified: false,
        result: None,
    };
    journal::write(&record, true)?;
    checkpoint("accepted")?;
    let mut completed_persisted = false;
    let attempt = publish(&mut record, &held, &checkpoint, &mut completed_persisted);
    match attempt {
        Ok(()) => Ok(record),
        Err(primary) => {
            if completed_persisted {
                return Err(primary);
            }
            record.state = if matches!(record.state, State::Accepted | State::Preparing) {
                State::FailedBeforePublication
            } else {
                State::Uncertain
            };
            record.result = None;
            if let Err(error) = journal::write(&record, false) {
                return Err(format!(
                    "{primary}; activation outcome could not be recorded: {error}"
                ));
            }
            Err(primary)
        }
    }
}

fn publish(
    record: &mut Record,
    held: &ActivationLease,
    checkpoint: &impl Fn(&str) -> Result<(), String>,
    completed_persisted: &mut bool,
) -> Result<(), String> {
    record.state = State::Preparing;
    journal::write(record, false)?;
    checkpoint("preparing")?;
    fs::create_dir_all(held.root.join("releases")).map_err(authority::display)?;
    authority::stage::publish(
        &record.command.bootstrap_directory,
        &held.root,
        &held.root.join("releases").join(&record.selected_release),
        record
            .command
            .expected_bootstrap_tree_sha256
            .as_deref()
            .ok_or("tree missing")?,
    )?;
    checkpoint("generationPublished")?;
    record.state = State::PublicationIntent;
    journal::write(record, false)?;
    checkpoint("publicationIntent")?;
    authority::select_release(&held.root, &record.selected_release)?;
    checkpoint("pointerPublished")?;
    authority::refresh_auto_update_helper(&held.root)?;
    record.automatic_update_helper_verified =
        super::status::helper_verified(&held.root, &record.selected_release);
    if !record.automatic_update_helper_verified {
        return Err("selected update helper was not verified".into());
    }
    let result = authority::result(
        &held.root,
        held,
        record.selected_release.clone(),
        record.command.expected_daemon_sha256.as_deref(),
    )?;
    if Some(result.bootstrap_tree_sha256.as_str())
        != record.command.expected_bootstrap_tree_sha256.as_deref()
    {
        return Err("activated tree changed before completion".into());
    }
    record.result = Some(result);
    record.state = State::Completed;
    journal::write(record, false)?;
    *completed_persisted = true;
    checkpoint("completedPersisted")?;
    if record.command.enable_automatic_updates {
        if let Err(e) = authority::enable_updates(&held.root, &held.data_dir) {
            eprintln!("Gent automatic-update scheduling was not refreshed: {e}");
        }
    }
    Ok(())
}
