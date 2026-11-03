use gent_protocol::runtime_activation::{ActivationCommand, ActivationOperationRecord};
use sha2::{Digest, Sha256};

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}
pub(super) fn operation_id(value: &str) -> Result<(), String> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| "invalid operation UUID")?;
    if id.is_nil() || id.hyphenated().to_string() != value {
        return Err("operation UUID must be canonical and nonzero".into());
    }
    Ok(())
}
pub(super) fn fingerprint(
    command: &ActivationCommand,
    platform: &str,
    version: &str,
) -> Result<String, String> {
    #[derive(serde::Serialize)]
    struct Bound<'a> {
        schema: u16,
        command: &'a ActivationCommand,
        platform: &'a str,
        version: &'a str,
    }
    let mut sha = Sha256::new();
    sha.update(b"gent-runtime-activation-command-v1\0");
    let mut identity_command = command.clone();
    identity_command.expected_command_sha256 = None;
    sha.update(
        serde_json::to_vec(&Bound {
            schema: 1,
            command: &identity_command,
            platform,
            version,
        })
        .map_err(|e| e.to_string())?,
    );
    Ok(hex::encode(sha.finalize()))
}
pub(super) fn validate(record: &ActivationOperationRecord) -> Result<(), String> {
    operation_id(&record.operation_id)?;
    let op = record
        .command
        .operation
        .as_ref()
        .ok_or("operation identity missing")?;
    if record.schema != 1
        || op.operation_id != record.operation_id
        || op.expected_owner.process_id == 0
        || !valid_digest(&op.expected_owner.executable_digest_sha256)
        || record.command.expected_host_epoch != Some(record.origin_host_epoch)
        || record.origin_host_epoch == 0
        || record.effect_issuer_process_id == 0
        || !valid_digest(&record.command.maintenance_daemon_sha256)
        || !record
            .command
            .expected_daemon_sha256
            .as_deref()
            .is_some_and(valid_digest)
        || !record
            .command
            .expected_bootstrap_tree_sha256
            .as_deref()
            .is_some_and(valid_digest)
        || !super::super::valid_version(&record.version)
        || record.platform != super::super::expected_target()?
        || fingerprint(&record.command, &record.platform, &record.version)? != record.command_sha256
    {
        return Err("durable activation identity is inconsistent".into());
    }
    if record.command.expected_command_sha256.as_deref() != Some(record.command_sha256.as_str()) {
        return Err("execution was not bound to its reviewed operation plan".into());
    }
    let tree = record
        .command
        .expected_bootstrap_tree_sha256
        .as_ref()
        .ok_or("tree missing")?;
    let expected = format!(
        "{}+{}.{}-{}",
        record.version,
        &tree[..16],
        record.command_sha256,
        record.platform
    );
    if record.selected_release != expected {
        return Err("operation generation identity changed".into());
    }
    let terminal = matches!(
        record.state,
        gent_protocol::runtime_activation::ActivationOperationState::Completed
            | gent_protocol::runtime_activation::ActivationOperationState::RecoveredPublication
    );
    if terminal != record.result.is_some() {
        return Err("durable activation terminal result is inconsistent".into());
    }
    if let Some(result) = &record.result {
        let executable = record
            .command
            .runtime_root
            .join("releases")
            .join(&record.selected_release)
            .join(if cfg!(windows) { "gentd.exe" } else { "gentd" });
        if result.schema != 1
            || result.daemon_path != executable
            || result.data_dir != record.command.data_directory
            || result.selected_release != record.selected_release
            || result.origin_host_epoch != Some(record.origin_host_epoch)
            || Some(result.executable_digest_sha256.as_str())
                != record.command.expected_daemon_sha256.as_deref()
            || Some(result.bootstrap_tree_sha256.as_str())
                != record.command.expected_bootstrap_tree_sha256.as_deref()
        {
            return Err(
                "durable activation terminal result differs from its accepted command".into(),
            );
        }
    }
    Ok(())
}
