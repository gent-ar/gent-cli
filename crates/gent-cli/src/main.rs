use std::path::PathBuf;

use clap::{Parser, Subcommand};
use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyPlanRequest, DependencyProvider, WireFrame,
};
use gent_types::{Command, ReceiptId};
use serde_json::Value;

mod decision;
mod local_ipc;

use crate::decision::{DecisionCommandLine, decision_frame};
use crate::local_ipc::request;

#[derive(Debug, Parser)]
#[command(name = "gent", about = "Protocol-only client for a local gentd")]
struct Args {
    #[arg(long, env = "GENT_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: CommandLine,
}

#[derive(Debug, Subcommand)]
enum CommandLine {
    /// Read-only dependency discovery through the local daemon.
    Doctor,
    /// Review or explicitly consent to a public provider dependency action.
    Deps {
        #[command(subcommand)]
        action: DependencyCommand,
    },
    /// Submit or terminally settle a durable provider-neutral decision.
    Decision {
        #[command(subcommand)]
        action: DecisionCommandLine,
    },
    Status,
    Submit {
        #[arg(long)]
        kind: String,
        #[arg(long, default_value = "{}")]
        payload: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    Events {
        #[arg(long, default_value_t = 0)]
        after_cursor: u64,
    },
}

#[derive(Debug, Subcommand)]
enum DependencyCommand {
    /// Show a read-only install or update plan.
    Plan {
        action: DependencyAction,
        provider: DependencyProvider,
    },
    /// Confirm an install plan. No installer is started until this capability is configured.
    Install {
        provider: DependencyProvider,
        #[arg(long)]
        consent: bool,
    },
    /// Confirm an update plan. No updater is started until this capability is configured.
    Update {
        provider: DependencyProvider,
        #[arg(long)]
        consent: bool,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    match args.command {
        CommandLine::Doctor => println!(
            "{}",
            serde_json::to_string_pretty(&request(args.data_dir, WireFrame::DoctorRequest).await?)?
        ),
        CommandLine::Deps { action } => {
            let frame = dependency_frame(&action);
            println!(
                "{}",
                serde_json::to_string_pretty(&request(args.data_dir, frame).await?)?
            );
        }
        CommandLine::Decision { action } => println!(
            "{}",
            serde_json::to_string_pretty(&request(args.data_dir, decision_frame(&action)).await?)?
        ),
        CommandLine::Status => println!(
            "{}",
            serde_json::to_string_pretty(&request(args.data_dir, WireFrame::StatusRequest).await?)?
        ),
        CommandLine::Submit {
            kind,
            payload,
            idempotency_key,
        } => {
            let status = request(args.data_dir.clone(), WireFrame::StatusRequest).await?;
            let WireFrame::Status(status) = status else {
                return Err("daemon did not return host status".into());
            };
            let payload: Value = serde_json::from_str(&payload)?;
            let command = Command {
                receipt_id: ReceiptId::new(),
                idempotency_key: idempotency_key.unwrap_or_else(|| ReceiptId::new().0),
                host_epoch: status.host_epoch,
                kind,
                payload,
            };
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &request(args.data_dir, WireFrame::Command(command)).await?
                )?
            );
        }
        CommandLine::Events { after_cursor } => println!(
            "{}",
            serde_json::to_string_pretty(
                &request(args.data_dir, WireFrame::Subscribe { after_cursor }).await?
            )?
        ),
    }
    Ok(())
}

fn dependency_frame(action: &DependencyCommand) -> WireFrame {
    match action {
        DependencyCommand::Plan { action, provider } => {
            WireFrame::DependencyPlanRequest(DependencyPlanRequest {
                provider: *provider,
                action: *action,
            })
        }
        DependencyCommand::Install { provider, consent } => {
            dependency_action(*provider, DependencyAction::Install, *consent)
        }
        DependencyCommand::Update { provider, consent } => {
            dependency_action(*provider, DependencyAction::Update, *consent)
        }
    }
}

fn dependency_action(
    provider: DependencyProvider,
    action: DependencyAction,
    consent_granted: bool,
) -> WireFrame {
    WireFrame::DependencyActionRequest(DependencyActionRequest {
        provider,
        action,
        consent_granted,
    })
}

#[cfg(test)]
mod tests {
    use gent_protocol::{DependencyAction, DependencyProvider, WireFrame};

    use super::{DependencyCommand, dependency_frame};

    #[test]
    fn dependency_plan_is_read_only() {
        assert!(matches!(
            dependency_frame(&DependencyCommand::Plan {
                action: DependencyAction::Install,
                provider: DependencyProvider::Claude,
            }),
            WireFrame::DependencyPlanRequest(_)
        ));
    }

    #[test]
    fn dependency_install_requires_explicit_consent_flag() {
        assert!(matches!(
            dependency_frame(&DependencyCommand::Install {
                provider: DependencyProvider::Codex,
                consent: false,
            }),
            WireFrame::DependencyActionRequest(request) if !request.consent_granted
        ));
    }
}
