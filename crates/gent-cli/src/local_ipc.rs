//! Local transport selection; command composition remains in `main`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use gent_protocol::{Hello, WireFrame, read_frame, write_frame};
use gent_types::{CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN};
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    frame: WireFrame,
) -> Result<WireFrame, Box<dyn std::error::Error>> {
    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    let mut stream = connect_or_start(&data_dir).await?;
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

#[cfg(unix)]
type LocalStream = UnixStream;
#[cfg(windows)]
type LocalStream = NamedPipeClient;

async fn connect_or_start(data_dir: &Path) -> Result<LocalStream, Box<dyn std::error::Error>> {
    match connect(data_dir).await {
        Ok(stream) => return Ok(stream),
        #[cfg(windows)]
        Err(error) if pipe_is_busy(&error) => return wait_for_connection(data_dir).await,
        Err(_) => {}
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
    wait_for_connection(data_dir).await
}

async fn wait_for_connection(data_dir: &Path) -> Result<LocalStream, Box<dyn std::error::Error>> {
    for _ in 0..40 {
        if let Ok(stream) = connect(data_dir).await {
            return Ok(stream);
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err("gentd did not become ready; set GENTD_BIN to the daemon executable".into())
}

#[cfg(unix)]
async fn connect(data_dir: &Path) -> Result<LocalStream, std::io::Error> {
    UnixStream::connect(data_dir.join("gentd.sock")).await
}

#[cfg(windows)]
#[allow(clippy::unused_async)] // Keeps the shared call site transport-agnostic.
async fn connect(data_dir: &Path) -> Result<LocalStream, std::io::Error> {
    ClientOptions::new().open(pipe_name(data_dir))
}

#[cfg(windows)]
fn pipe_is_busy(error: &std::io::Error) -> bool {
    // ERROR_PIPE_BUSY from the Win32 API. Tokio's client docs prescribe retrying it.
    error.raw_os_error() == Some(231)
}

fn default_daemon_binary() -> PathBuf {
    let name = if cfg!(windows) { "gentd.exe" } else { "gentd" };
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("ar", "Gent", "Gent").map_or_else(
        || PathBuf::from(".gent"),
        |directories| directories.data_local_dir().to_path_buf(),
    )
}

#[cfg(windows)]
fn pipe_name(data_dir: &Path) -> String {
    format!(r"\\.\pipe\gentd-{:016x}", endpoint_hash(data_dir))
}

#[cfg(windows)]
fn endpoint_hash(data_dir: &Path) -> u64 {
    data_dir
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}
