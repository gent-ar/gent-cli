use gent_protocol::runtime_activation::{
    ACTIVATION_MAINTENANCE_MAX_BYTES, ActivationMaintenanceReply as Reply,
    ActivationMaintenanceRequest as Request,
};
use std::io::{Read, Write};

pub(crate) fn requested() -> Result<bool, String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|a| a == "--runtime-activation-stdio")
    {
        if args.len() != 1 {
            return Err("maintenance does not accept daemon startup flags".into());
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

pub(crate) fn run_stdio() -> Result<(), String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(ACTIVATION_MAINTENANCE_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let reply = if bytes.len() > ACTIVATION_MAINTENANCE_MAX_BYTES {
        failure(
            "maintenanceRequestTooLarge",
            "Maintenance request exceeds its bound",
        )
    } else {
        match serde_json::from_slice::<Request>(&bytes) {
            Ok(request) => execute(request),
            Err(_) => failure(
                "maintenanceInvalidRequest",
                "Invalid bounded maintenance request",
            ),
        }
    };
    let bytes = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
    if bytes.len() > ACTIVATION_MAINTENANCE_MAX_BYTES {
        return Err("maintenance reply exceeds bound".into());
    }
    std::io::stdout()
        .write_all(&bytes)
        .map_err(|e| e.to_string())
}

fn execute(request: Request) -> Reply {
    if request.schema != 1 {
        return failure("maintenanceUnsupported", "Unknown maintenance schema");
    }
    let expected = &request.expected_maintenance_daemon_sha256;
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return failure(
            "maintenanceIdentityRequired",
            "A trusted daemon digest is required",
        );
    }
    if let gent_protocol::runtime_activation::ActivationMaintenanceAction::Activate(command)
    | gent_protocol::runtime_activation::ActivationMaintenanceAction::Plan(command) =
        &request.action
    {
        if &command.maintenance_daemon_sha256 != expected {
            return failure(
                "maintenanceIdentityMismatch",
                "Command authority differs from pinned issuer",
            );
        }
    }
    let identity = std::env::current_exe()
        .map_err(|e| e.to_string())
        .and_then(|p| crate::runtime_activation::operation::self_digest(&p));
    match identity {
        Ok(actual) if actual.eq_ignore_ascii_case(expected) => {
            crate::runtime_activation::operation::dispatch(request.action)
                .unwrap_or_else(|e| failure("maintenanceRejected", &e))
        }
        _ => failure(
            "maintenanceIdentityMismatch",
            "Executing daemon differs from caller's trusted digest",
        ),
    }
}

fn failure(code: &str, message: &str) -> Reply {
    Reply::Error {
        code: code.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gent_protocol::runtime_activation::ActivationMaintenanceAction as Action;
    #[test]
    fn unsupported_schema_and_wrong_pinned_identity_never_reach_actions() {
        let action = Action::BootstrapIdentity {
            bootstrap_directory: "must-not-be-created".into(),
        };
        let reply = execute(Request {
            schema: 9,
            expected_maintenance_daemon_sha256: "0".repeat(64),
            action: action.clone(),
        });
        assert!(matches!(reply, Reply::Error { code, .. } if code == "maintenanceUnsupported"));
        let reply = execute(Request {
            schema: 1,
            expected_maintenance_daemon_sha256: "0".repeat(64),
            action,
        });
        assert!(
            matches!(reply, Reply::Error { code, .. } if code == "maintenanceIdentityMismatch")
        );
        assert!(!std::path::Path::new("must-not-be-created").exists());
    }
}
