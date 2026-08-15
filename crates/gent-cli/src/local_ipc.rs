//! Local transport selection; command composition remains in `main`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use gent_protocol::{
    CONVERSATION_ACTIVITY_CAPABILITY, CONVERSATION_INDEX_CAPABILITY,
    CONVERSATION_STATUS_CAPABILITY, CONVERSATION_TIMELINE_CAPABILITY, EVENT_STREAM_CAPABILITY,
    Hello, WireFrame, read_frame, write_frame,
};
use gent_types::{CapabilitySet, PROTOCOL_MAX, PROTOCOL_MIN};

#[cfg(unix)]
use gent_protocol::CONVERSATION_CONTENT_CAPABILITY;
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

pub(crate) async fn request(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    frame: WireFrame,
) -> Result<WireFrame, Box<dyn std::error::Error>> {
    let (mut stream, _) = connect_and_negotiate(data_dir, no_autostart).await?;
    write_frame(&mut stream, &frame).await?;
    let response = read_frame(&mut stream).await?;
    if let WireFrame::Error { message, .. } = &response {
        return Err(message.clone().into());
    }
    Ok(response)
}

/// Opens local IPC and requires the mandatory protocol handshake before any extension frame.
pub(crate) async fn connect_and_negotiate(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
) -> Result<(LocalStream, CapabilitySet), Box<dyn std::error::Error>> {
    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    let mut stream = connect_or_start(&data_dir, no_autostart).await?;
    write_frame(
        &mut stream,
        &WireFrame::Hello(Hello {
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: client_capabilities(),
        }),
    )
    .await?;
    match read_frame(&mut stream).await? {
        WireFrame::Negotiated(answer) => Ok((stream, answer.capabilities)),
        WireFrame::Error { message, .. } => Err(message.into()),
        _ => Err("daemon did not negotiate protocol".into()),
    }
}

#[must_use]
pub(crate) fn client_capabilities() -> CapabilitySet {
    #[cfg(unix)]
    let mut capabilities = vec![
        CONVERSATION_ACTIVITY_CAPABILITY.into(),
        CONVERSATION_INDEX_CAPABILITY.into(),
        CONVERSATION_STATUS_CAPABILITY.into(),
        CONVERSATION_TIMELINE_CAPABILITY.into(),
        "decisions".into(),
        "event-resync".into(),
        EVENT_STREAM_CAPABILITY.into(),
        "events".into(),
        "host-epoch".into(),
        "receipts".into(),
    ];
    #[cfg(not(unix))]
    let capabilities = vec![
        CONVERSATION_ACTIVITY_CAPABILITY.into(),
        CONVERSATION_INDEX_CAPABILITY.into(),
        CONVERSATION_STATUS_CAPABILITY.into(),
        CONVERSATION_TIMELINE_CAPABILITY.into(),
        "decisions".into(),
        "event-resync".into(),
        EVENT_STREAM_CAPABILITY.into(),
        "events".into(),
        "host-epoch".into(),
        "receipts".into(),
    ];
    #[cfg(unix)]
    capabilities.push(CONVERSATION_CONTENT_CAPABILITY.into());
    CapabilitySet(capabilities)
}

#[cfg(unix)]
type LocalStream = UnixStream;
#[cfg(windows)]
type LocalStream = NamedPipeClient;

pub(crate) async fn connect_or_start(
    data_dir: &Path,
    no_autostart: bool,
) -> Result<LocalStream, Box<dyn std::error::Error>> {
    match connect(data_dir).await {
        Ok(stream) => return Ok(stream),
        #[cfg(windows)]
        Err(error) if pipe_is_busy(&error) => return wait_for_connection(data_dir).await,
        Err(_) => {}
    }
    if no_autostart {
        return Err("gentd is unavailable and --no-autostart was requested".into());
    }
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

#[cfg(all(test, unix))]
#[path = "local_ipc_tests.rs"]
mod tests;
