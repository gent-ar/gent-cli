//! `gent update` command dispatch separate from the terminal composition root.

use std::path::PathBuf;

use crate::{
    auto_update_handoff, local_ipc, runtime_maintenance, runtime_update_check,
    update_check::UpdateCommand, update_handoff,
};

pub(crate) async fn execute(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    action: UpdateCommand,
) -> Result<Option<serde_json::Value>, Box<dyn std::error::Error>> {
    match action {
        UpdateCommand::Auto { action } => {
            auto_update_handoff::invoke(
                &action,
                data_dir.unwrap_or_else(local_ipc::default_data_dir),
            )?;
            Ok(None)
        }
        UpdateCommand::Status { attempt_id } => Ok(Some(serde_json::to_value(
            runtime_maintenance::request(data_dir, no_autostart, attempt_id).await?,
        )?)),
        UpdateCommand::Check { channel } => Ok(Some(serde_json::to_value(
            runtime_update_check::request(data_dir, no_autostart, channel.into()).await?,
        )?)),
        UpdateCommand::Apply {
            version,
            expected_sha256,
            consent,
            install_dir,
        } => {
            if !consent {
                return Err("runtime updates require --consent".into());
            }
            update_handoff::apply(&update_handoff::UpdateRequest {
                version,
                expected_sha256,
                data_dir: data_dir.unwrap_or_else(local_ipc::default_data_dir),
                install_dir,
            })?;
            Ok(None)
        }
    }
}
