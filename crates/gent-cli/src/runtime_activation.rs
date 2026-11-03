use gent_protocol::runtime_activation::{
    ACTIVATION_MAINTENANCE_MAX_BYTES, ActivationMaintenanceReply, ActivationMaintenanceRequest,
};
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};
#[path = "runtime_activation_command.rs"]
mod command;
pub(crate) use command::RuntimeCommand;
#[path = "runtime_activation_delegate.rs"]
mod delegate;

pub(crate) fn execute(command: RuntimeCommand, data_dir: PathBuf) -> Result<String, String> {
    let (pin, mut action, json) = command::request(command, data_dir)?;
    let expected = pin
        .expected_maintenance_daemon_sha256
        .ok_or_else(|| {
            error(
                "maintenanceIdentityRequired",
                "A trusted daemon digest is required",
            )
        })?
        .to_ascii_lowercase();
    let executable = delegate::pinned_sibling(&expected)?;
    if let gent_protocol::runtime_activation::ActivationMaintenanceAction::Activate(command)
    | gent_protocol::runtime_activation::ActivationMaintenanceAction::Plan(command) = &mut action
    {
        command.maintenance_daemon_sha256.clone_from(&expected);
    }
    let request = ActivationMaintenanceRequest {
        schema: 1,
        expected_maintenance_daemon_sha256: expected,
        action,
    };
    let input = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    if input.len() > ACTIVATION_MAINTENANCE_MAX_BYTES {
        return Err(error(
            "maintenanceRequestTooLarge",
            "Maintenance request exceeds its bound",
        ));
    }
    let mut child = Command::new(executable)
        .arg("--runtime-activation-stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| error("maintenanceUnavailable", &e.to_string()))?;
    let stdout = child.stdout.take().ok_or("maintenance stdout missing")?;
    let stderr = child.stderr.take().ok_or("maintenance stderr missing")?;
    let output = std::thread::spawn(move || delegate::bounded_output(stdout));
    let diagnostic = std::thread::spawn(move || delegate::bounded_output(stderr));
    let write = child
        .stdin
        .take()
        .ok_or("maintenance stdin missing")?
        .write_all(&input);
    let exit = child.wait().map_err(|e| e.to_string())?;
    let output = output
        .join()
        .map_err(|_| "maintenance output reader failed")?;
    let _diagnostic = diagnostic
        .join()
        .map_err(|_| "maintenance diagnostic reader failed")?;
    if !exit.success() {
        return Err(error(
            "maintenanceUnsupported",
            "Pinned daemon did not complete the maintenance protocol",
        ));
    }
    write.map_err(|e| e.to_string())?;
    let reply: ActivationMaintenanceReply = serde_json::from_slice(&output?).map_err(|_| {
        error(
            "maintenanceUnsupported",
            "Pinned daemon returned no valid maintenance reply",
        )
    })?;
    match reply {
        ActivationMaintenanceReply::Error { code, message } => Err(error(&code, &message)),
        ActivationMaintenanceReply::Capabilities {
            schema,
            capabilities,
        } => {
            serde_json::to_string(&serde_json::json!({"schema":schema,"capabilities":capabilities}))
                .map_err(|e| e.to_string())
        }
        ActivationMaintenanceReply::BootstrapIdentity(value) => {
            serde_json::to_string(&value).map_err(|e| e.to_string())
        }
        ActivationMaintenanceReply::Operation(value) => {
            serde_json::to_string(&value).map_err(|e| e.to_string())
        }
        ActivationMaintenanceReply::Planned(value) => {
            serde_json::to_string(&value).map_err(|e| e.to_string())
        }
        ActivationMaintenanceReply::Origin(value) => {
            serde_json::to_string(&value).map_err(|e| e.to_string())
        }
        ActivationMaintenanceReply::Activated(value) if json => {
            serde_json::to_string(&value).map_err(|e| e.to_string())
        }
        ActivationMaintenanceReply::Activated(value) => Ok(value.daemon_path.display().to_string()),
    }
}
fn error(code: &str, message: &str) -> String {
    serde_json::json!({"code":code,"message":message}).to_string()
}
#[cfg(test)]
#[path = "runtime_activation_delegate_tests.rs"]
mod tests;
