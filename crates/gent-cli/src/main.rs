use std::path::PathBuf;
use std::process::Stdio;

use clap::{Parser, Subcommand};
use gent_protocol::{Hello, WireFrame, read_frame, write_frame};
use gent_types::{
    CapabilitySet, Command, DependencyStatus, DoctorReport, PROTOCOL_MAX, PROTOCOL_MIN, ReceiptId,
};
use serde_json::Value;
use tokio::net::UnixStream;

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
    /// Read-only dependency discovery; it never installs or configures a provider.
    Doctor,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    match args.command {
        CommandLine::Doctor => println!("{}", serde_json::to_string_pretty(&doctor().await)?),
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

async fn doctor() -> DoctorReport {
    let mut dependencies = Vec::new();
    for (name, remediation) in [
        (
            "claude",
            "Install explicitly with `gent deps install claude` or the vendor installer.",
        ),
        (
            "codex",
            "Install explicitly with `gent deps install codex` or the vendor installer.",
        ),
        (
            "node",
            "Install Node.js explicitly before enabling MCP features.",
        ),
    ] {
        let version = tokio::process::Command::new(name)
            .arg("--version")
            .output()
            .await
            .ok()
            .and_then(|output| {
                output.status.success().then(|| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_owned()
                })
            });
        dependencies.push(DependencyStatus {
            name: name.into(),
            present: version.is_some(),
            version,
            remediation: remediation.into(),
        });
    }
    DoctorReport { dependencies }
}
