use std::path::PathBuf;
use std::process::Stdio;

use clap::{Parser, Subcommand};
use gent_protocol::{
    DependencyAction, DependencyActionRequest, DependencyPlanRequest, DependencyProvider, Hello,
    WireFrame, read_frame, write_frame,
};
use gent_types::{CapabilitySet, Command, PROTOCOL_MAX, PROTOCOL_MIN, ReceiptId};
use serde_json::Value;
use tokio::net::UnixStream;

mod decision;

use crate::decision::{DecisionCommandLine, decision_frame};

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

async fn request(
    data_dir: Option<PathBuf>,
    frame: WireFrame,
) -> Result<WireFrame, Box<dyn std::error::Error>> {
    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    let socket = data_dir.join("gentd.sock");
    let mut stream = connect_or_start(&socket, &data_dir).await?;
    write_frame(
        &mut stream,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: CapabilitySet(vec![
                "decisions".into(),
                "events".into(),
                "host-epoch".into(),
                "receipts".into(),
            ]),
        }),
    )
    .await?;
    match read_frame(&mut stream).await? {
        WireFrame::Negotiated(_) => {}
        WireFrame::Error { message, .. } => return Err(message.into()),
        _ => return Err("daemon did not negotiate protocol".into()),
    }
    write_frame(&mut stream, &frame).await?;
    let response = read_frame(&mut stream).await?;
    if let WireFrame::Error { message, .. } = &response {
        return Err(message.clone().into());
    }
    Ok(response)
}

async fn connect_or_start(
    socket: &PathBuf,
    data_dir: &PathBuf,
) -> Result<UnixStream, Box<dyn std::error::Error>> {
    if let Ok(stream) = UnixStream::connect(socket).await {
        return Ok(stream);
    }
    std::fs::create_dir_all(data_dir)?;
    let daemon = std::env::var_os("GENTD_BIN").map_or_else(default_daemon_binary, PathBuf::from);
    tokio::process::Command::new(daemon)
        .arg("--data-dir")
        .arg(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    for _ in 0..40 {
        if let Ok(stream) = UnixStream::connect(socket).await {
            return Ok(stream);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err("gentd did not become ready; set GENTD_BIN to the daemon executable".into())
}

fn default_daemon_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join("gentd")))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("gentd"))
}

fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("ar", "Gent", "Gent").map_or_else(
        || PathBuf::from(".gent"),
        |directories| directories.data_local_dir().to_path_buf(),
    )
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
