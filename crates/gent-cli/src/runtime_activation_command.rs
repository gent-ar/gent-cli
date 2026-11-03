use clap::{Args, Subcommand};
use gent_protocol::runtime_activation::{
    ActivationCommand, ActivationLookup, ActivationMaintenanceAction as Action,
    ActivationOperationExpectation, ExpectedActivationOwner,
};
use std::path::PathBuf;

#[derive(Debug, Args)]
pub(crate) struct IdentityPin {
    #[arg(long, help = "SHA-256 the maintenance daemon must match")]
    pub(crate) expected_maintenance_daemon_sha256: Option<String>,
}
#[derive(Debug, Args)]
pub(crate) struct Lookup {
    #[command(flatten)]
    pin: IdentityPin,
    #[arg(long, help = "Runtime installation root")]
    runtime_root: PathBuf,
    #[arg(long, help = "Data directory of the idle daemon")]
    idle_data_dir: PathBuf,
    #[arg(long, help = "Activation operation id")]
    operation_id: String,
    #[arg(long, help = "SHA-256 of the activation command")]
    command_sha256: String,
}
#[derive(Debug, Args)]
pub(crate) struct Origin {
    #[command(flatten)]
    pin: IdentityPin,
    #[arg(long, help = "Runtime installation root")]
    runtime_root: PathBuf,
    #[arg(long, help = "Data directory of the idle daemon")]
    idle_data_dir: PathBuf,
    #[arg(long, help = "Host epoch the daemon must still hold")]
    expected_host_epoch: u64,
}
#[derive(Debug, Args)]
pub(crate) struct Activate {
    #[command(flatten)]
    pin: IdentityPin,
    #[arg(long, help = "Bootstrap directory of the release to install")]
    bootstrap_dir: PathBuf,
    #[arg(long, help = "Runtime installation root")]
    runtime_root: Option<PathBuf>,
    #[arg(long, help = "Data directory of the idle daemon")]
    idle_data_dir: Option<PathBuf>,
    #[arg(long, help = "SHA-256 the installed daemon must match")]
    expected_daemon_sha256: Option<String>,
    #[arg(long, help = "SHA-256 the bootstrap tree must match")]
    expected_bootstrap_tree_sha256: Option<String>,
    #[arg(long, help = "Host epoch the daemon must still hold")]
    expected_host_epoch: Option<u64>,
    #[arg(long, help = "Activation operation id")]
    operation_id: Option<String>,
    #[arg(
        long,
        requires = "operation_id",
        help = "SHA-256 the resumed command must match"
    )]
    expected_command_sha256: Option<String>,
    #[arg(
        long,
        requires = "operation_id",
        help = "Process id of the expected owner"
    )]
    expected_owner_process_id: Option<u32>,
    #[arg(
        long,
        requires = "operation_id",
        help = "SHA-256 of the expected owner daemon"
    )]
    expected_owner_daemon_sha256: Option<String>,
    #[arg(long, help = "Print the result as JSON")]
    json: bool,
}
#[derive(Debug, Subcommand)]
pub(crate) enum RuntimeCommand {
    #[command(about = "Install and activate a runtime release")]
    Activate(Activate),
    #[command(about = "Show the activation plan without applying it")]
    ActivationPlan(Activate),
    #[command(about = "Report the activation capabilities of this runtime")]
    ActivationCapabilities(IdentityPin),
    #[command(about = "Print the identity of a bootstrap directory")]
    BootstrapIdentity {
        #[command(flatten)]
        pin: IdentityPin,
        #[arg(long, help = "Bootstrap directory of the release to install")]
        bootstrap_dir: PathBuf,
    },
    #[command(about = "Read the status of an activation operation")]
    ActivationStatus(Lookup),
    #[command(about = "Adopt an activation operation left by another owner")]
    ActivationAdopt(Lookup),
    #[command(about = "Read the status of the daemon an activation replaced")]
    ActivationOriginStatus(Origin),
}
pub(crate) fn request(
    command: RuntimeCommand,
    data: PathBuf,
) -> Result<(IdentityPin, Action, bool), String> {
    Ok(match command {
        RuntimeCommand::ActivationCapabilities(pin) => (pin, Action::Capabilities, true),
        RuntimeCommand::ActivationOriginStatus(v) => (
            v.pin,
            Action::OriginStatus(gent_protocol::runtime_activation::ActivationOriginLookup {
                runtime_root: v.runtime_root,
                data_directory: v.idle_data_dir,
                expected_host_epoch: v.expected_host_epoch,
            }),
            true,
        ),
        RuntimeCommand::BootstrapIdentity { pin, bootstrap_dir } => (
            pin,
            Action::BootstrapIdentity {
                bootstrap_directory: bootstrap_dir,
            },
            true,
        ),
        RuntimeCommand::ActivationStatus(value) => {
            let (pin, lookup) = lookup(value);
            (pin, Action::Status(lookup), true)
        }
        RuntimeCommand::ActivationAdopt(value) => {
            let (pin, lookup) = lookup(value);
            (pin, Action::Adopt(lookup), true)
        }
        command @ (RuntimeCommand::Activate(_) | RuntimeCommand::ActivationPlan(_)) => {
            let planning = matches!(&command, RuntimeCommand::ActivationPlan(_));
            let (RuntimeCommand::Activate(v) | RuntimeCommand::ActivationPlan(v)) = command else {
                unreachable!()
            };
            let operation = match v.operation_id {
                Some(operation_id) => Some(ActivationOperationExpectation {
                    operation_id,
                    expected_owner: ExpectedActivationOwner {
                        process_id: v
                            .expected_owner_process_id
                            .ok_or("expected owner PID is required")?,
                        executable_digest_sha256: v
                            .expected_owner_daemon_sha256
                            .ok_or("expected owner daemon digest is required")?,
                    },
                }),
                None if v.expected_owner_process_id.is_some()
                    || v.expected_owner_daemon_sha256.is_some() =>
                {
                    return Err("owner identity requires an operation ID".into());
                }
                None => None,
            };
            let automatic = v.runtime_root.is_none();
            let body = ActivationCommand {
                bootstrap_directory: v.bootstrap_dir,
                runtime_root: v.runtime_root.unwrap_or_else(default_root),
                data_directory: v.idle_data_dir.unwrap_or(data),
                expected_daemon_sha256: v.expected_daemon_sha256,
                expected_bootstrap_tree_sha256: v.expected_bootstrap_tree_sha256,
                expected_host_epoch: v.expected_host_epoch,
                operation,
                expected_command_sha256: v.expected_command_sha256,
                maintenance_daemon_sha256: String::new(),
                enable_automatic_updates: automatic,
            };
            (
                v.pin,
                if planning {
                    Action::Plan(body)
                } else {
                    Action::Activate(body)
                },
                v.json,
            )
        }
    })
}
fn lookup(v: Lookup) -> (IdentityPin, ActivationLookup) {
    (
        v.pin,
        ActivationLookup {
            operation_id: v.operation_id,
            command_sha256: v.command_sha256,
            runtime_root: v.runtime_root,
            data_directory: v.idle_data_dir,
        },
    )
}
fn default_root() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Gent")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/lib/gent")
    }
}
